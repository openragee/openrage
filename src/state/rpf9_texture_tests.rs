mod install {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use or_io::codec::texture::decode_level;
    use or_io::gta6::hash::name_hash64;
    use or_io::gta6::headerless::Resource;

    use crate::state::document::rpf9::{entry::EntrySource, Rpf9TableDocument};
    use crate::state::{texture_resolution, AppState, DocPayload, Document, Prefs};

    fn install_root() -> Option<PathBuf> {
        let Some(root) = std::env::var_os("OPENRAGE_GTA6_BUILD") else {
            or_test_support::skip_install(
                "RPF9 external texture resolution",
                format_args!("set OPENRAGE_GTA6_BUILD and OPENRAGE_GTA6_KEYS"),
            );
            return None;
        };
        Some(root.into())
    }

    fn entry(root: &Path, archive: &str, chain: &[u32]) -> EntrySource {
        let table = Rpf9TableDocument::open(&root.join(archive))
            .expect("the archive table opens")
            .expect("an RPF9 archive");
        let mut base = 0;
        for &index in &chain[..chain.len() - 1] {
            base = table
                .expand_table(base, index)
                .expect("the selected child table opens");
        }
        table
            .entry(base, *chain.last().expect("an entry index"))
            .expect("the selected table entry exists")
    }

    #[test]
    fn a_real_ydr_resolves_external_texture_keys_from_an_auto_opened_table_ytd() {
        let Some(root) = install_root() else {
            return;
        };
        let _serial = crate::ui::theme::serial();
        const ENTRY_LIMIT: u64 = 1 << 20;
        let model_source = entry(&root, "data.rpf", &[26, 2]);
        let dictionary_source = entry(&root, "textures.rpf", &[1, 0]);
        assert!(model_source.decoded_len() + dictionary_source.decoded_len() <= ENTRY_LIMIT);
        let model = Document::new(
            1,
            Some(model_source.path.clone()),
            model_source
                .payload_limited(ENTRY_LIMIT)
                .expect("the selected YDR opens in memory"),
        );
        let DocPayload::Gta6Drawable(resource) = &model.payload else {
            panic!("the table YDR must use the GTA VI reader");
        };
        let Resource::Drawable(drawable) = &**resource else {
            panic!("the selected entry must be a drawable");
        };
        let keys: BTreeSet<u64> = drawable
            .shaders
            .iter()
            .flat_map(|shader| shader.texture_keys.iter().flatten().copied())
            .collect();
        let expected = ["blank_mb", "blank_ma", "blank_nm"];
        for name in expected {
            assert!(keys.contains(&name_hash64(name.as_bytes())), "{name}");
        }
        let wanted: Vec<_> = keys
            .iter()
            .map(|key| (None, format!("hash_{key:016X}")))
            .collect();
        let mut state = AppState::from_prefs(Prefs::default());
        state.documents.push(model);
        state.active_document = Some(1);
        let before = texture_resolution::report(&state, None, wanted.clone(), 0);
        assert_eq!(before.resolved(), 0, "these textures are external");

        let dictionary = Document::new(
            2,
            Some(dictionary_source.path.clone()),
            dictionary_source
                .payload_limited(ENTRY_LIMIT)
                .expect("the selected YTD opens in memory"),
        );
        assert!(matches!(
            &dictionary.payload,
            DocPayload::Resource(or_io::Asset::TextureDict(_))
        ));
        state.documents.push(dictionary);
        state.auto_opened_texture_docs.insert(2);
        for document in &state.documents {
            assert_eq!(
                document.read_only,
                Some(crate::state::document::rpf9::entry::READ_ONLY)
            );
            assert!(crate::state::ttpf_document::is_gta6(Some(document)));
            assert!(!document.path.as_ref().expect("table address").exists());
        }
        let after = texture_resolution::report(&state, None, wanted, 1);
        assert!(after.resolved() >= expected.len());
        for name in expected {
            let key = name_hash64(name.as_bytes());
            let wanted = format!("hash_{key:016X}");
            let resolved = texture_resolution::resolve(&state, None, None, &wanted)
                .unwrap_or_else(|| panic!("the YTD must supply {name}"));
            assert_eq!(resolved.origin.document, 2);
            assert!(resolved.origin.implicitly_opened);
            assert_eq!(resolved.texture.name(), name);
            assert!(resolved.texture.has_pixel_data());
            let image = decode_level(&resolved.texture, 0).expect("preview pixels decode");
            assert!(image.width > 0 && image.height > 0);
            assert_eq!(
                image.data.len(),
                image.width as usize * image.height as usize * 4
            );
        }
        assert!(!model_source.path.exists());
        assert!(!dictionary_source.path.exists());
    }

    #[test]
    fn two_existing_loose_ydrs_open_through_their_byte_identical_level_table_entries() {
        use std::fs::File;
        use std::io::Read;

        use or_job::{Cancel, Progress};

        use crate::state::rpf9_texture_index::{Root, Tables};

        let Some(root) = install_root() else {
            return;
        };
        let Some(loose) = std::env::var_os("OPENRAGE_GTA6_LOOSE_YDR_DIR") else {
            or_test_support::skip_install(
                "loose GTA VI drawable table association",
                format_args!("set OPENRAGE_GTA6_LOOSE_YDR_DIR to the existing exported YDR folder"),
            );
            return;
        };
        let loose = PathBuf::from(loose);
        let archive = root.join("levels.rpf");
        let cancel = Cancel::new();
        let tables = Tables::open(vec![Root::File(archive.clone())], &cancel, &Progress::new())
            .expect("the table catalog was not cancelled");
        assert!(!tables.truncated, "the level tables fit the catalog budget");
        assert_eq!(tables.failed, 0, "{:?}", tables.failures);
        const ENTRY_LIMIT: u64 = 1 << 20;
        for (name, address) in [
            ("int_abd_1_rt_office_decals2", "1544_0"),
            ("int_bnk_1_gfa_bespoke_vec", "340_10"),
        ] {
            let hash = name_hash64(name.as_bytes());
            let original = loose.join(format!("hash_{hash:016x}.ydr"));
            let mut bytes = Vec::new();
            File::open(&original)
                .expect("the existing exported drawable is available")
                .take(ENTRY_LIMIT + 1)
                .read_to_end(&mut bytes)
                .expect("the existing drawable can be read");
            assert!(bytes.len() as u64 <= ENTRY_LIMIT);
            let renamed = loose.join(format!("preview-test-{hash:016x}.ydr"));
            assert!(!renamed.exists());
            let opened = crate::state::rpf9_loose::open(&renamed, &bytes, &tables, &cancel)
                .expect("the loose drawable matches a table entry")
                .expect("the bytes identify a GTA VI drawable");
            assert_eq!(
                crate::state::document::rpf9::entry::archive_path(&opened.source.path),
                Some(archive.as_path())
            );
            assert_eq!(
                opened
                    .source
                    .path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|part| part.to_str()),
                Some(address)
            );
            assert_eq!(
                opened.source.bytes_limited(ENTRY_LIMIT).unwrap(),
                bytes,
                "the table supplies the section sizes only for identical bytes"
            );
            let DocPayload::Gta6Drawable(resource) = opened.payload else {
                panic!("a matching loose YDR must use the GTA VI preview");
            };
            let Resource::Drawable(drawable) = &*resource else {
                panic!("the loose resource must be a drawable");
            };
            assert_eq!(drawable.name, format!("{name}.#dr"));
            assert!(
                !renamed.exists(),
                "preview must not write the loose resource"
            );
        }
    }
}
