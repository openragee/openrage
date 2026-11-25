use std::time::{Duration, Instant};

use super::super::opening_an_rpf9_archive::{rpf9_head, scanned, written, zlib};
use super::*;
use crate::state::rpf9_scan::body_path;

/// A lone animation set named `name`, of at most 15 bytes, with no animation, as a virtual section.
fn lone_set(name: &str) -> Vec<u8> {
    let mut root = vec![0u8; 0xC0];
    root[0x38..0x38 + name.len()].copy_from_slice(name.as_bytes());
    root[0x47] = 0x80 | name.len() as u8;
    root[0x5F] = 1;
    root[0x68..0x6C]
        .copy_from_slice(&or_io::gta6::hash::name_hash32(name.as_bytes()).to_le_bytes());
    root[0x08..0x10].copy_from_slice(&0x5000_00B0u64.to_le_bytes());
    root[0xB8] = 1;
    root
}

/// A clip dictionary that names the sets of `keys` as stored in other bodies.
fn naming(keys: &[u32]) -> AnimResource {
    AnimResource {
        root: RootKind::ClipDictionary,
        root_word: 2,
        name: "sf_dict".into(),
        key: 0,
        block_map: None,
        word_at_60: 0,
        word_at_98: None,
        sets: Vec::new(),
        external_sets: keys.to_vec(),
        animation_keys: Vec::new(),
        clips: Vec::new(),
        motions: Vec::new(),
        matchers: Vec::new(),
        dictionaries: Vec::new(),
        end_region: None,
        sections: None,
        edited: false,
    }
}

fn finished(state: &mut AppState) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !state.set_finding.is_empty() {
        state.poll_set_finding();
        assert!(Instant::now() < deadline, "the search never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_set_another_body_stores_is_found_among_the_animation_bodies_of_the_scan() {
    let _serial = crate::ui::theme::serial();
    let key = |name: &str| or_io::gta6::hash::name_hash32(name.as_bytes());
    let mut bytes = rpf9_head(1, 0x1A0);
    for name in ["sf_test-1", "sf_test-2"] {
        bytes.resize(bytes.len().next_multiple_of(0x1000) + 16, 0);
        bytes.extend_from_slice(&zlib(&lone_set(name)));
    }
    let dir = tempfile::tempdir().expect("a scratch directory");
    let archive = written(dir.path(), &bytes);
    let mut state = AppState::new();
    scanned(&mut state, archive.clone());
    let wanted = [key("sf_test-2"), key("sf_test-9")];
    let path = body_path(&archive, 0x10, View::Animation);
    let doc = state.insert_document(
        path,
        DocPayload::Gta6Anim(Arc::new(naming(&wanted))),
        None,
        None,
    );
    assert!(found_set(&archive, wanted[0]).is_none());

    state.find_gta6_sets(doc).expect("a scan to look in");
    assert_eq!(state.set_finding.len(), 1, "the search is a job");
    finished(&mut state);
    let set = found_set(&archive, wanted[0]).expect("the set the scan holds");
    assert_eq!(set.name, "sf_test-2");
    assert!(
        found_set(&archive, key("sf_test-1")).is_none(),
        "a set nothing names is not kept"
    );
    assert!(found_set(&archive, wanted[1]).is_none());
    let said = state
        .jobs
        .ended()
        .iter()
        .find(|j| j.kind == JobKind::SetSearch)
        .expect("its end");
    assert!(said.said.contains("1 of 2"), "{}", said.said);

    // Another archive's dictionary naming the same set is searched in its own archive.
    std::fs::create_dir(dir.path().join("b")).expect("a sub folder");
    let other = written(&dir.path().join("b"), &bytes);
    scanned(&mut state, other.clone());
    assert!(found_set(&other, wanted[0]).is_none());
    let there = state.insert_document(
        body_path(&other, 0x10, View::Animation),
        DocPayload::Gta6Anim(Arc::new(naming(&wanted[..1]))),
        None,
        None,
    );
    state.find_gta6_sets(there).expect("a scan to look in");
    assert_eq!(state.set_finding.len(), 1, "the other archive is searched");
    finished(&mut state);
    assert!(found_set(&other, wanted[0]).is_some());

    // A dictionary that is no scanned body has no scan to look in.
    let elsewhere = state.insert_document(
        "C:/anims/a.anim".into(),
        DocPayload::Gta6Anim(Arc::new(naming(&[key("sf_test-8")]))),
        None,
        None,
    );
    assert!(state.find_gta6_sets(elsewhere).is_err());
}

#[test]
fn a_table_animation_never_starts_an_external_set_scan() {
    let _serial = crate::ui::theme::serial();
    let mut state = AppState::new();
    let path = "animations.rpf/.rpf9-table/0/hash_external.ycd".into();
    state.documents.push(super::super::Document::new(
        1,
        Some(path),
        DocPayload::Gta6Anim(Arc::new(naming(&[or_io::gta6::hash::name_hash32(
            b"table_missing_set",
        )]))),
    ));
    assert!(state.document(1).unwrap().is_from_rpf9_table());
    state
        .find_gta6_sets(1)
        .expect("table previews never request a scan");
    assert!(state.set_finding.is_empty());
}
