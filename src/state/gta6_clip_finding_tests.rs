use std::time::{Duration, Instant};

use or_io::gta6::anim::{Motion, RootKind};

use super::super::gta6_anim_edit::tests::dictionary;
use super::super::opening_an_rpf9_archive::{rpf9_head, scanned, settle, written, zlib};
use super::*;

/// A motion dictionary of one motion that names the clips of `keys`.
fn naming(keys: &[u64]) -> AnimResource {
    AnimResource {
        root: RootKind::MotionDictionary,
        root_word: 0,
        name: "cf_motions".into(),
        key: 0,
        block_map: None,
        word_at_60: 0,
        word_at_98: None,
        sets: Vec::new(),
        external_sets: Vec::new(),
        animation_keys: Vec::new(),
        clips: Vec::new(),
        motions: vec![Motion {
            name: "cf_motion".into(),
            key: 0,
            kind: 1,
            clips: keys.to_vec(),
            time_warps: Vec::new(),
            weight_lists: Vec::new(),
            parameters: Vec::new(),
            parameter_items: Vec::new(),
            groups: Vec::new(),
            parameter_objects: Vec::new(),
            properties: Vec::new(),
        }],
        matchers: Vec::new(),
        dictionaries: Vec::new(),
        end_region: None,
        sections: None,
        edited: false,
    }
}

#[test]
fn a_clip_a_motion_names_is_found_among_the_clip_dictionaries_of_the_open_scans_and_opens() {
    let _serial = crate::ui::theme::serial();
    let section = dictionary();
    let held = or_io::gta6::anim::read(&section).expect("it reads");
    let clip = held.clips.first().expect("a clip").clone();
    // A key no body holds, which differs from the clip's in both halves.
    let missing = !clip.key;
    let mut bytes = rpf9_head(1, 0x1A0);
    bytes.resize(bytes.len().next_multiple_of(0x1000) + 16, 0);
    bytes.extend_from_slice(&zlib(&section));
    let dir = tempfile::tempdir().expect("a scratch directory");
    let archive = written(dir.path(), &bytes);
    let mut state = AppState::new();
    let motions = state.insert_document(
        "C:/anims/cf.motions".into(),
        DocPayload::Gta6Anim(Arc::new(naming(&[clip.key, missing]))),
        None,
        None,
    );
    assert!(
        state.find_gta6_clips(motions).is_err(),
        "no scan is open to look in"
    );
    assert!(found_clip(clip.key).is_none());

    scanned(&mut state, archive.clone());
    state.find_gta6_clips(motions).expect("a scan to look in");
    assert!(state.clip_finding.is_some(), "the search is a job");
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.clip_finding.is_some() {
        state.poll_set_finding();
        assert!(Instant::now() < deadline, "the search never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
    let found = found_clip(clip.key).expect("the clip the scan holds");
    assert_eq!(found.archive, archive);
    assert_eq!(found.clip, clip.name);
    assert_eq!(found.dictionary, held.name);
    assert!(found_clip(missing).is_none(), "a key no body holds");
    let said = state
        .jobs
        .ended()
        .iter()
        .find(|j| j.kind == JobKind::SetSearch)
        .expect("its end");
    assert!(said.said.contains("1 of 2"), "{}", said.said);

    // Its dictionary opens from where the search found it.
    state.open_found_clip(&found);
    settle(&mut state);
    let opened = state.active_document.and_then(|d| state.document(d));
    assert!(
        matches!(
            opened.map(|d| &d.payload),
            Some(DocPayload::Gta6Anim(r)) if r.clips.iter().any(|c| c.key == clip.key)
        ),
        "the clip's dictionary did not open: {}",
        state.error_dialog_message
    );
}

#[test]
fn a_table_motion_dictionary_never_starts_a_clip_scan() {
    let _serial = crate::ui::theme::serial();
    let mut state = AppState::new();
    let path = "animations.rpf/.rpf9-table/0/hash_motion.ycd".into();
    state.documents.push(super::super::Document::new(
        1,
        Some(path),
        DocPayload::Gta6Anim(Arc::new(naming(&[0x1010_2020_3030_4040]))),
    ));
    assert!(state.document(1).unwrap().is_from_rpf9_table());
    state
        .find_gta6_clips(1)
        .expect("table previews never request a scan");
    assert!(state.clip_finding.is_none());
}
