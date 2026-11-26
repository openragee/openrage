//! A GTA VI dictionary that holds one drawable opens with it selected, as a `.ydd` opens on its
//! first entry; one of several drawables waits for a choice.

use std::sync::Arc;

use or_io::gta6::headerless::{Dictionary, Drawable, Entry, Resource};

use super::{AppState, DocPayload};
use crate::ui::panels::selection_ids::dict_entry_id;

fn drawable(name: &str) -> Entry {
    Entry::Drawable(Box::new(Drawable {
        name: name.to_string(),
        box_min: [0.0; 3],
        box_max: [1.0; 3],
        shaders: Vec::new(),
        levels: [None, None, None, None],
        lod_distances: [0.0; 4],
        bound: None,
        embedded: None,
        extra_skeletons: Vec::new(),
    }))
}

fn opened(entries: Vec<(u32, Entry)>) -> (AppState, Option<u64>) {
    let entries = entries.into_iter().map(|(code, e)| (code, Ok(e))).collect();
    let mut state = AppState::new();
    let payload = DocPayload::Gta6Drawable(Arc::new(Resource::Dictionary(Dictionary { entries })));
    let doc = state.insert_document("0x40.dictionary".into(), payload, None, None);
    let selected = state.selection.selected(doc);
    (state, selected)
}

#[test]
fn a_dictionary_of_one_drawable_opens_on_it_and_one_of_two_on_none() {
    // A word beside the one drawable is not a second model.
    let (state, selected) = opened(vec![(0x10, Entry::Word64(7)), (0x20, drawable("a.#dr"))]);
    assert_eq!(selected, Some(dict_entry_id(0x20)));
    let doc = state.active_document.expect("open");
    let Some(DocPayload::Gta6Drawable(resource)) = state.document(doc).map(|d| &d.payload) else {
        panic!("a GTA VI body");
    };
    let drawn = super::gta6_resource::drawable_of(resource, selected);
    assert_eq!(drawn.map(|d| d.name.as_str()), Some("a.#dr"));

    let (_, selected) = opened(vec![(0x10, drawable("a.#dr")), (0x20, drawable("b.#dr"))]);
    assert_eq!(selected, None, "two drawables wait for a choice");
    let (_, selected) = opened(vec![(0x10, Entry::Word64(7))]);
    assert_eq!(selected, None, "no drawable, nothing to open on");
}
