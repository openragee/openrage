use super::*;
use crate::state::Prefs;
use or_io::codec::texture::GrcTextureFormat;
use or_io::gta5::{gen8::GrcTexture, PgDictionary};

#[test]
fn a_table_texture_preview_cannot_save_as_a_gta5_resource_or_change_its_source_path() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("scratch directory");
    let source = dir.path().join("textures.rpf/.rpf9-table/0/entry");
    let destination = dir.path().join("copy.ytd");
    let mut dictionary = PgDictionary::default();
    dictionary.insert(GrcTexture {
        name: "table_paint".into(),
        width: 4,
        height: 4,
        depth: 1,
        format: GrcTextureFormat::A8B8G8R8.to_raw_format(),
        mip_count: 1,
        backing_store: vec![0x71; 4 * 4 * 4],
        ..Default::default()
    });
    let before = dictionary.clone();
    let mut state = AppState::from_prefs(Prefs::default());
    let id = state.insert_document(
        source.clone(),
        DocPayload::Resource(or_io::Asset::TextureDict(dictionary)),
        None,
        None,
    );
    assert_eq!(state.document(id).unwrap().read_only, Some(READ_ONLY));
    assert!(!state.save_active_document());
    assert!(!state.save_active_document_as(destination.clone()));
    assert!(state.save_receiver.is_none());
    assert!(state.pending_retail_edit.is_none());
    let document = state.document(id).expect("preview stays open");
    assert_eq!(document.path.as_ref(), Some(&source));
    assert_eq!(document.read_only, Some(READ_ONLY));
    assert!(!document.dirty);
    assert!(!document.history.can_undo());
    assert_eq!(
        crate::state::textures::legacy_dictionary(&document.payload),
        Some(&before)
    );
    assert!(!destination.exists());
    assert!(!source.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn a_table_preview_never_builds_a_gta5_image_and_refuses_scalar_edits_to_one() {
    use or_editor::{Edit, EditKind};
    use or_io::desc::{IntWidth, ScalarValue};

    let _serial = crate::ui::theme::serial();
    let mut state = AppState::from_prefs(Prefs::default());
    let path = PathBuf::from("textures.rpf/.rpf9-table/0/entry");
    let id = state.insert_document(
        path,
        DocPayload::Resource(or_io::Asset::TextureDict(Default::default())),
        None,
        None,
    );
    let document = state.document(id).expect("inserted preview");
    assert_eq!(document.read_only, Some(READ_ONLY));
    assert!(document.resource.is_none());
    assert!(crate::state::described_root(document).is_none());
    document.ask_for_image();
    state.seal_edits();
    assert!(state.document(id).unwrap().resource.is_none());
    assert!(crate::state::described_root(state.document(id).unwrap()).is_none());
    state.seal_edits();
    assert!(state.document(id).unwrap().resource.is_none());

    // An image carried by an older preview must not make its fields editable again.
    let bytes = or_io::Asset::TextureDict(Default::default())
        .to_bytes()
        .expect("synthetic resource");
    let image = crate::state::document_io::resource_image_for(DocumentKind::Ytd, &bytes)
        .expect("synthetic image");
    let before = image.data.clone();
    state.document_mut(id).unwrap().set_resource(Some(image));
    assert!(crate::state::described_root(state.document(id).unwrap()).is_none());
    let edit = Edit {
        offset: 0,
        kind: EditKind::Int(IntWidth::U32),
    };
    assert!(!state.patch_scalar(id, "Synthetic field", edit, ScalarValue::Int(123)));
    assert!(!state.patch_scalars(
        id,
        "Synthetic fields".into(),
        &[(edit, ScalarValue::Int(456))]
    ));
    let document = state.document(id).unwrap();
    assert_eq!(document.resource.as_ref().unwrap().data, before);
    assert!(!document.dirty);
    assert!(!document.history.can_undo());
    assert!(!document.history.can_redo());
}

#[test]
fn an_explicit_scanned_dictionary_keeps_its_existing_image_description() {
    let _serial = crate::ui::theme::serial();
    let mut state = AppState::from_prefs(Prefs::default());
    let id = state.insert_document(
        PathBuf::from("scanned_dictionary.ytd"),
        DocPayload::Resource(or_io::Asset::TextureDict(Default::default())),
        None,
        None,
    );
    state.document_mut(id).unwrap().read_only = Some(crate::state::rpf9_scan::READ_ONLY_BODY);
    assert!(crate::state::described_root(state.document(id).unwrap()).is_none());
    state.seal_edits();
    assert!(state.document(id).unwrap().resource.is_some());
    assert!(crate::state::described_root(state.document(id).unwrap()).is_some());
}
