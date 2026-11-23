use or_io::gta6::hash::name_hash64;

use super::*;
use crate::state::jobs::JobKind;
use crate::state::opening_a_ttpf::{named_by_the_rules, open, pack, written};

/// What the pack's first texture is shown as.
fn first_name(state: &AppState, doc: crate::state::DocumentId) -> String {
    match &state.document(doc).expect("open").payload {
        DocPayload::Ttpf(ttpf) => ttpf.name(0),
        _ => panic!("a .ttpf payload"),
    }
}

#[test]
fn a_names_file_names_the_key_its_line_hashes_to_and_says_how_many_it_named() {
    let name = "zq_names_file_glow";
    let key = name_hash64(name.as_bytes());
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut bytes = pack(0xF800);
    // The first texture's key, after the header, the table and the two directory entries.
    let keys = 0x100 + 16 + 16 * 2;
    bytes[keys..keys + 8].copy_from_slice(&key.to_le_bytes());
    let mut state = AppState::new();
    let doc = open(&mut state, written(dir.path(), &bytes));
    assert_eq!(first_name(&state, doc), format!("hash_{key:016X}"));

    let names = dir.path().join("names.txt");
    let lines = format!("\r\nzq_names_file_nothing\r\n   {name}  \r\n");
    std::fs::write(&names, lines).expect("written");
    state.load_texture_names(&names);
    named_by_the_rules(&mut state);
    assert_eq!(first_name(&state, doc), name);
    let said = (state.jobs.ended().iter())
        .filter(|job| job.kind == JobKind::TextureNames)
        .map(|job| job.said.clone())
        .collect::<Vec<_>>();
    assert!(
        said.iter()
            .any(|s| s == "1 GTA VI texture key named from the names file"),
        "{said:?}"
    );
}

#[test]
fn a_names_file_with_no_pack_open_starts_nothing_and_says_why() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let names = dir.path().join("names.txt");
    std::fs::write(&names, "zq_glow\n").expect("written");
    let mut state = AppState::new();
    state.load_texture_names(&names);
    assert!(state.texture_naming.is_empty(), "no pack, no job");
    assert!(
        (state.toasts.iter()).any(|t| t.message.contains("Open a GTA VI texture pack first")),
        "{:?}",
        state.toasts
    );
}

#[test]
fn a_key_nothing_learned_this_session_is_named_by_the_vocabulary_a_harvest_kept() {
    let name = "zq_kept_by_a_harvest_glow";
    let key = name_hash64(name.as_bytes());
    let mut kept = or_io::gta6::names::Vocabulary::default();
    assert!(kept.add(name, None));
    let named_by = crate::state::ttpf_document::named_by;
    assert_eq!(named_by(None, &kept, key).as_deref(), Some(name));
    // What the session learned comes first, and a key neither holds has no name.
    let learned = Some("zq_typed".to_string());
    assert_eq!(named_by(learned, &kept, key).as_deref(), Some("zq_typed"));
    assert_eq!(named_by(None, &kept, key ^ 1), None);
}
