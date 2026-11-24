use super::super::tests::{keys, wait};
use super::*;
use crate::state::{textures, Document, Prefs};
use or_io::codec::texture::{decode_level, TextureSource};
use or_io::container::rpf::{
    Gta6EntryType, Rpf9Builder, Rpf9Sections, Rpf9Toc, GROUP_ENTRY_ROW_20,
};
use or_io::gta6::headerless::{write_dictionary, DictionaryModel, ModelTexture};

fn sections() -> Rpf9Sections {
    let paint = ModelTexture::tiled("table_paint", 64, 64, 71, &[vec![0x5a; 2048]])
        .expect("synthetic BC1 texture");
    let mut array = ModelTexture::tiled("table_array", 8, 8, 28, &[vec![0x37; 256]])
        .expect("synthetic array slice");
    array.depth = 2;
    array.payload = array.payload.repeat(usize::from(array.depth));
    let mut cube = ModelTexture::tiled("table_cube", 4, 4, 28, &[vec![0x29; 64]])
        .expect("synthetic cube face");
    cube.dimension = 2;
    cube.depth = 6;
    cube.payload = cube.payload.repeat(usize::from(cube.depth));
    let volume = ModelTexture {
        name: "table_volume".into(),
        width: 8,
        height: 16,
        depth: 8,
        dimension: 3,
        format: 28,
        mip_levels: 1,
        flags: 0x980,
        byte_43: 0,
        byte_45: 0,
        payload: vec![0; 0x1000],
    };
    write_dictionary(&DictionaryModel {
        textures: vec![paint, array, cube, volume],
        capacity: 4,
    })
    .expect("synthetic dictionary")
}

fn encrypted(bytes: Vec<u8>) -> Vec<u8> {
    Rpf9Builder::serialize(
        &Rpf9Toc {
            bytes,
            key_id: 0,
            registry_id: 0,
            entry_count: 0,
            groups: Vec::new(),
        },
        &keys(),
    )
    .expect("synthetic encrypted table")
}

fn dictionary_archive(
    sections: &Rpf9Sections,
    resource_type: u32,
) -> (tempfile::TempDir, Arc<Rpf9TableDocument>, Vec<u8>) {
    let body = [
        sections.virtual_section.as_slice(),
        sections.physical.as_slice(),
    ]
    .concat();
    let count = u32::try_from(body.len()).unwrap();
    let mut toc = Rpf9Toc::decrypt(
        &encrypted(or_test_support::rpf9::resource_table(&[(
            count, count, false,
        )])),
        &keys(),
    )
    .unwrap();
    let row = toc
        .groups
        .iter()
        .find(|group| group.name_hash == GROUP_ENTRY_ROW_20)
        .unwrap()
        .data
        .start;
    for (offset, value) in [
        (0, u32::try_from(sections.virtual_section.len()).unwrap()),
        (4, u32::try_from(sections.physical.len()).unwrap() | 3),
        (16, resource_type),
    ] {
        toc.bytes[row + offset..row + offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let mut child = Rpf9Builder::serialize(&toc, &keys()).unwrap();
    child.resize(0x1010, 0);
    child.extend(&body);
    let mut bytes = encrypted(or_test_support::rpf9::table(&[(
        or_io::gta6::hash::name_hash64(b"table_child"),
        0x1000,
        child.len() as u64,
        0,
    )]));
    bytes.resize(0x1000, 0);
    bytes.extend(child);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("textures.rpf");
    std::fs::write(&path, bytes).unwrap();
    let table = Arc::new(
        Rpf9TableDocument::open_using(&path, || Ok(keys()))
            .unwrap()
            .unwrap(),
    );
    table.toggle(0, 0);
    wait(&table);
    (dir, table, body)
}

fn preview_count(payload: &DocPayload) -> usize {
    let textures::TextureListing::Textures {
        textures: rows,
        source,
    } = textures::textures_in(payload, None)
    else {
        panic!("dictionary must expose the texture preview");
    };
    assert!(!rows.is_empty());
    for row in &rows {
        let texture = textures::texture_at(payload, source, &row.name)
            .expect("listed texture has preview pixels");
        assert!(texture.has_pixel_data(), "{}", row.name);
        for level in 0..u32::from(texture.mip_count()) {
            let image = decode_level(&texture, level)
                .unwrap_or_else(|error| panic!("{} mip {level}: {error}", row.name));
            assert_eq!(
                image.data.len(),
                image.width as usize * image.height as usize * 4
            );
        }
    }
    rows.len()
}

#[test]
fn a_table_dictionary_opens_reloads_and_previews_in_memory() {
    let _serial = crate::ui::theme::serial();
    let (dir, table, expected) = dictionary_archive(&sections(), 5);
    let source = table.entry(0x1000, 0).unwrap();
    assert!(source.name.starts_with("hash_"));
    assert!(Path::new(&source.name).extension().is_none());
    assert!(!source.path.exists());
    assert_eq!(
        table.tree().entry_type(0x1000, 0),
        (Gta6EntryType::TextureDictionary, false)
    );
    assert!(!table.tree().errors.contains_key(&(0x1000, 0)));
    let payload = source.payload().expect("typed dictionary");
    assert_eq!(
        table.tree().entry_type(0x1000, 0),
        (Gta6EntryType::TextureDictionary, true)
    );
    assert!(!table.tree().errors.contains_key(&(0x1000, 0)));
    assert_eq!(preview_count(&payload), 4);
    let document = Document::new(1, Some(source.path.clone()), payload);
    assert_eq!(document.read_only, Some(READ_ONLY));
    assert!(crate::state::ttpf_document::is_gta6(Some(&document)));
    let DocPayload::Resource(or_io::Asset::TextureDict(dictionary)) = &document.payload else {
        panic!("table dictionary must enter the existing texture preview");
    };
    for (name, width, height, slices, image_type, pixels) in [
        ("table_paint", 64, 64, 1, 0, vec![0x5a; 2048]),
        ("table_array", 8, 8, 2, 0, vec![0x37; 512]),
        ("table_cube", 4, 4, 6, 1, vec![0x29; 384]),
        (
            "table_volume",
            8,
            16,
            8,
            crate::state::ttpf_document::VOLUME_IMAGE,
            vec![0; 4096],
        ),
    ] {
        let texture = dictionary
            .entries
            .iter()
            .flatten()
            .find(|t| t.name == name)
            .unwrap();
        assert_eq!(
            (texture.width, texture.height, texture.image_type),
            (width, height, image_type)
        );
        assert_eq!(texture.payload(), pixels);
        assert_eq!(
            crate::state::ttpf_document::scanned_slices(&document, texture),
            slices
        );
    }
    let reopened = Rpf9TableDocument::open_using(&table.path, || Ok(keys()))
        .unwrap()
        .unwrap();
    let reloaded = read_chain(&reopened, &[0, 0]).unwrap().payload().unwrap();
    let DocPayload::Resource(or_io::Asset::TextureDict(reloaded)) = reloaded else {
        panic!("reload must preserve the typed preview");
    };
    assert_eq!(&reloaded, dictionary);

    let mut state = AppState::from_prefs(Prefs::default());
    state.open_rpf9_entry(&table, 0x1000, 0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while state.load_receiver.is_some() {
        state.poll_load_result();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let opened = state
        .active_document()
        .expect("worker opened the dictionary");
    assert_eq!(opened.read_only, Some(READ_ONLY));
    assert_eq!(preview_count(&opened.payload), 4);
    let mut extracted = Vec::new();
    assert_eq!(
        source.extract(&mut extracted).unwrap(),
        expected.len() as u64
    );
    assert_eq!(
        extracted, expected,
        "preview conversion must not change extraction"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(!source.path.exists(), "opening creates no extracted file");
}

#[test]
fn a_queued_table_preview_confirms_the_type_in_the_open_parent_tree() {
    let _serial = crate::ui::theme::serial();
    let (dir, table, _) = dictionary_archive(&sections(), 5);
    let source = table.entry(0x1000, 0).unwrap();
    let mut state = AppState::from_prefs(Prefs::default());
    state.insert_document(
        table.path.clone(),
        DocPayload::Rpf9(crate::state::Rpf9Document::Table(Arc::clone(&table))),
        None,
        None,
    );
    let blocked_path = dir.path().join("already_loading.bin");
    let (tx, rx) = mpsc::channel();
    state.load_receiver = Some((blocked_path.clone(), rx));
    state.open_rpf9_entry(&table, 0x1000, 0);
    assert_eq!(state.pending_loads.front(), Some(&source.path));
    assert_eq!(
        table.tree().entry_type(0x1000, 0),
        (Gta6EntryType::TextureDictionary, false)
    );
    tx.send(LoadResult::Success {
        path: blocked_path,
        payload: DocPayload::Hex(crate::state::HexDocument {
            bytes: vec![0x78],
            banner: String::new(),
        }),
        binding: None,
        resource: None,
    })
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while state.load_receiver.is_some() {
        state.poll_load_result();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let opened = state.active_document().expect("queued preview opened");
    assert_eq!(opened.path.as_ref(), Some(&source.path));
    assert_eq!(preview_count(&opened.payload), 4);
    assert_eq!(opened.read_only, Some(READ_ONLY));
    assert_eq!(
        table.tree().entry_type(0x1000, 0),
        (Gta6EntryType::TextureDictionary, true)
    );
    assert!(state.pending_loads.is_empty());
    assert!(!source.path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn a_partly_unreadable_dictionary_is_an_error_without_a_scan_or_partial_preview() {
    let mut sections = sections();
    sections.physical.truncate(sections.physical.len() - 0x1000);
    let dictionary = or_io::gta6::headerless::read_dictionary(
        &sections.virtual_section,
        Some(&sections.physical),
    )
    .unwrap();
    let readable = dictionary.entries.iter().filter(|(_, entry)| {
        matches!(entry, Ok(or_io::gta6::headerless::Entry::Texture(t)) if t.texture.is_ok())
    }).count();
    assert!(readable > 0 && readable < dictionary.entries.len());
    let (dir, table, expected) = dictionary_archive(&sections, 5);
    let source = table.entry(0x1000, 0).unwrap();
    assert_eq!(
        table.tree().entry_type(0x1000, 0),
        (Gta6EntryType::TextureDictionary, false)
    );
    assert!(!table.tree().errors.contains_key(&(0x1000, 0)));
    assert!(
        source.payload().is_err(),
        "one invalid texture must refuse the preview"
    );
    assert_eq!(
        table.tree().entry_type(0x1000, 0),
        (Gta6EntryType::TextureDictionary, false)
    );
    assert!(table.tree().errors.contains_key(&(0x1000, 0)));
    let mut extracted = Vec::new();
    source.extract(&mut extracted).unwrap();
    assert_eq!(
        extracted, expected,
        "type detection cannot change archive bytes"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn a_dictionary_signature_of_another_table_type_does_not_become_a_texture_preview() {
    let (_dir, table, expected) = dictionary_archive(&sections(), 4);
    let source = table.entry(0x1000, 0).unwrap();
    let DocPayload::Hex(hex) = source.payload().unwrap() else {
        panic!("a shared dictionary signature cannot override the table resource type");
    };
    assert_eq!(hex.bytes, expected);
}

#[test]
fn a_preview_refuses_distinct_texture_names_that_share_a_hash() {
    use or_io::gta6::headerless::{read_dictionary, Dictionary, Entry};

    let sections = sections();
    let dictionary = read_dictionary(&sections.virtual_section, Some(&sections.physical)).unwrap();
    let names = ["table_collision_72000", "table_collision_88067"];
    assert_ne!(names[0], names[1]);
    assert_eq!(
        or_io::hash::string_hash_str(names[0]),
        or_io::hash::string_hash_str(names[1])
    );
    let entries = dictionary
        .entries
        .into_iter()
        .take(2)
        .zip(names)
        .map(|((_, entry), name)| {
            let Entry::Texture(mut texture) = entry.unwrap() else {
                panic!("synthetic texture");
            };
            assert!(texture.texture.is_ok());
            texture.name = name.into();
            (
                or_io::gta6::hash::name_hash32(name.as_bytes()),
                Ok(Entry::Texture(texture)),
            )
        })
        .collect();
    assert!(crate::state::ttpf_document::table_textures(Dictionary { entries }).is_err());
}

mod install {
    use super::*;

    #[test]
    fn selected_entries_across_five_archives_preview_and_reload_without_extracted_files() {
        let Some(root) = std::env::var_os("OPENRAGE_GTA6_PREVIEW_INSTALL") else {
            or_test_support::skip_install(
                "RPF9 table texture previews",
                format_args!("set OPENRAGE_GTA6_PREVIEW_INSTALL and OPENRAGE_GTA6_KEYS"),
            );
            return;
        };
        let snapshot = std::env::var_os("OPENRAGE_GTA6_KEYS").expect("OPENRAGE_GTA6_KEYS");
        let mut textures = 0;
        for (archive, chain, expected) in [
            ("textures.rpf", &[0, 0][..], 7),
            ("textures.rpf", &[1, 0][..], 12),
            ("textures.rpf", &[18, 0][..], 2),
            ("textures.rpf", &[2, 0][..], 1),
            ("textures.rpf", &[3, 0][..], 16),
            ("textures.rpf", &[20][..], 0),
            ("textures.rpf", &[21][..], 0),
            ("data.rpf", &[19, 8][..], 2),
            ("levels.rpf", &[1460, 231][..], 1),
            ("levels.rpf", &[637, 0][..], 1),
            (
                "pack/core_restofworld/core_restofworld.rpf",
                &[4, 100][..],
                1,
            ),
            (
                "pack/shared_minigame000/shared_minigame000.rpf",
                &[17, 1][..],
                2,
            ),
            ("levels.rpf", &[1815, 102][..], 0),
            ("levels.rpf", &[270, 2345][..], 0),
            ("levels.rpf", &[1087, 0][..], 0),
            ("levels.rpf", &[1418, 1044][..], 0),
            (
                "pack/shared_minigame000/shared_minigame000.rpf",
                &[29, 7][..],
                0,
            ),
        ] {
            let path = Path::new(&root).join(archive);
            let table = Rpf9TableDocument::open_using(&path, || {
                GtaViKeys::from_file(Path::new(&snapshot)).map_err(|error| error.to_string())
            })
            .unwrap()
            .unwrap();
            let source = read_chain(&table, chain).unwrap();
            let payload = source
                .payload()
                .unwrap_or_else(|error| panic!("{archive}:{chain:?}: {error}"));
            let bytes = source.bytes().unwrap();
            if expected == 0 {
                let DocPayload::Hex(hex) = payload else {
                    panic!("{archive}:{chain:?} must remain a byte preview");
                };
                assert_eq!(hex.bytes, bytes);
            } else {
                assert_eq!(preview_count(&payload), expected, "{archive}:{chain:?}");
                let document = Document::new(1, Some(source.path.clone()), payload);
                assert_eq!(document.read_only, Some(READ_ONLY));
                assert!(!source.path.exists());
                let reloaded = read_path(&source.path).unwrap();
                let (DocPayload::Resource(original), DocPayload::Resource(reloaded)) =
                    (&document.payload, reloaded)
                else {
                    panic!("{archive}:{chain:?} must reload as a texture dictionary");
                };
                assert_eq!(original, &reloaded);
                textures += expected;
            }
            let mut extracted = Vec::new();
            source.extract(&mut extracted).unwrap();
            assert_eq!(extracted, bytes, "{archive}:{chain:?}");
        }
        assert_eq!(textures, 45);
    }

    fn walk_previews(
        table: &Rpf9TableDocument,
        base: u64,
        totals: &mut [usize; 3],
        failures: &mut Vec<String>,
    ) {
        let entries = table.tree().nodes[&base].archive.entries.clone();
        for entry in entries {
            let (selection, texture_dictionary) = {
                let tree = table.tree();
                let node = &tree.nodes[&base];
                let selection = node
                    .chain
                    .iter()
                    .chain(std::iter::once(&entry.index))
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join("/");
                let texture_dictionary = node
                    .archive
                    .entry_extra(entry.index as usize)
                    .and_then(|extra| extra.nested)
                    .is_some_and(|row| row.resource_type == 5);
                (selection, texture_dictionary)
            };
            let label = format!("{}:{selection}", table.path.display());
            if entry.kind() == Some(Gta6Class::NestedArchive) {
                let child = match child_extent(&table.tree().nodes[&base], entry.index) {
                    Ok(child) => child,
                    Err(error) => {
                        failures.push(format!("{label}: {error}"));
                        continue;
                    }
                };
                table.toggle(base, entry.index);
                wait(table);
                {
                    let tree = table.tree();
                    if !tree.nodes.contains_key(&child.start) {
                        failures.push(format!(
                            "{label}: {:?}",
                            tree.errors.get(&(base, entry.index))
                        ));
                        continue;
                    }
                }
                walk_previews(table, child.start, totals, failures);
            } else if texture_dictionary {
                let opened = table
                    .entry(base, entry.index)
                    .and_then(|source| source.payload().map(|payload| (source, payload)));
                let (source, payload) = match opened {
                    Ok(opened) => opened,
                    Err(error) => {
                        failures.push(format!("{label}: {error}"));
                        continue;
                    }
                };
                let document = Document::new(1, Some(source.path.clone()), payload);
                assert_eq!(document.read_only, Some(READ_ONLY), "{label}");
                let DocPayload::Resource(or_io::Asset::TextureDict(dictionary)) = &document.payload
                else {
                    panic!("{label}: the table dictionary did not open its texture preview");
                };
                let textures::TextureListing::Textures {
                    textures: rows,
                    source: texture_source,
                } = textures::textures_in(&document.payload, None)
                else {
                    panic!("{label}: the texture preview has no listing");
                };
                assert_eq!(rows.len(), dictionary.entries.len(), "{label}");
                for row in rows {
                    let texture =
                        textures::texture_at(&document.payload, texture_source, &row.name)
                            .unwrap_or_else(|| panic!("{label}: no preview for {}", row.name));
                    assert!(texture.has_pixel_data(), "{label}: {}", row.name);
                }
                for texture in &dictionary.entries {
                    let texture = texture.as_ref().unwrap_or_else(|| {
                        panic!("{label}: the preview contains a missing texture")
                    });
                    totals[2] +=
                        crate::state::ttpf_document::scanned_slices(&document, texture) as usize;
                }
                totals[0] += 1;
                totals[1] += dictionary.entries.len();
                assert!(
                    !source.path.exists(),
                    "{label}: opening created an extracted file"
                );
            }
        }
    }

    #[test]
    fn every_table_texture_dictionary_in_the_selected_archive_previews_in_memory() {
        let Some(archive) = std::env::var_os("OPENRAGE_GTA6_ARCHIVE") else {
            or_test_support::skip_install(
                "All RPF9 table texture previews",
                format_args!("set OPENRAGE_GTA6_ARCHIVE and OPENRAGE_GTA6_KEYS"),
            );
            return;
        };
        let snapshot = std::env::var_os("OPENRAGE_GTA6_KEYS").expect("OPENRAGE_GTA6_KEYS");
        let table = Rpf9TableDocument::open_using(Path::new(&archive), || {
            GtaViKeys::from_file(Path::new(&snapshot)).map_err(|error| error.to_string())
        })
        .unwrap()
        .unwrap();
        let mut totals = [0; 3];
        let mut failures = Vec::new();
        walk_previews(&table, 0, &mut totals, &mut failures);
        println!(
            "{} dictionaries, {} textures and {} slices previewed from {} tables in memory; {} failures",
            totals[0],
            totals[1],
            totals[2],
            table.tree().nodes.len(),
            failures.len()
        );
        assert!(
            failures.is_empty(),
            "{} failures:\n{}",
            failures.len(),
            failures.join("\n")
        );
        assert!(
            totals[0] > 0,
            "the selected archive contains no texture dictionaries"
        );
    }
}
