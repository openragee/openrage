use super::super::tests::{fixture, keys, wait};
use super::*;
use crate::state::{Document, Prefs};

fn opened() -> (tempfile::TempDir, Arc<Rpf9TableDocument>) {
    let (dir, path) = fixture(0x1000, true);
    let doc = Arc::new(
        Rpf9TableDocument::open_using(&path, || Ok(keys()))
            .unwrap()
            .unwrap(),
    );
    (dir, doc)
}

#[test]
fn a_nested_file_opens_and_extracts_the_same_exact_bytes() {
    let _serial = crate::ui::theme::serial();
    let (dir, table) = opened();
    table.toggle(0, 0);
    wait(&table);
    let source = table.entry(0x1000, 0).unwrap();
    assert_eq!(source.bytes().unwrap(), vec![0x78; 16]);
    let (archive, chain) = address(&source.path).unwrap();
    assert_eq!(archive, table.path);
    assert_eq!(chain, [0, 0]);
    let reloaded = Rpf9TableDocument::open_using(&archive, || Ok(keys()))
        .unwrap()
        .unwrap();
    assert_eq!(
        read_chain(&reloaded, &chain).unwrap().bytes().unwrap(),
        source.bytes().unwrap()
    );

    let mut state = AppState::from_prefs(Prefs::default());
    state.open_rpf9_entry(&table, 0x1000, 0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while state.load_receiver.is_some() {
        state.poll_load_result();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let doc = state.active_document().unwrap();
    assert_eq!(doc.read_only, Some(READ_ONLY));
    assert_eq!(doc.file_name(), source.name);
    assert!(source.name.ends_with(".bin"));
    let DocPayload::Hex(hex) = &doc.payload else {
        panic!("entry was not opened as bytes")
    };
    assert_eq!(hex.bytes, source.bytes().unwrap());
    state.open_rpf9_entry(&table, 0x1000, 0);
    assert_eq!(
        state.documents.len(),
        1,
        "opening twice selects the same tab"
    );

    let dest = dir.path().join("out");
    std::fs::create_dir(&dest).unwrap();
    std::fs::write(dest.join(&source.name), b"keep me").unwrap();
    assert!(state.extract_rpf9_entry(&table, 0x1000, 0, dest.clone()));
    while state.extract_jobs.is_running() {
        state.poll_extract_job();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(std::fs::read(dest.join(&source.name)).unwrap(), b"keep me");
    assert_eq!(
        std::fs::read(dest.join(format!(
            "{} (2).bin",
            Path::new(&source.name).file_stem().unwrap().to_string_lossy()
        )))
        .unwrap(),
        source.bytes().unwrap()
    );
}

#[test]
fn a_changed_archive_fails_open_and_extraction_without_writing_a_file() {
    use crate::state::extract_job::{run_extraction, ExtractItem, OnClash};
    let (dir, table) = opened();
    let source = table.entry(0, 1).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&table.path)
        .unwrap()
        .set_len(0x3000)
        .unwrap();
    assert!(source.payload().is_err());
    let dest = dir.path().join("out");
    let item = ExtractItem::rpf9(source);
    let report = run_extraction(
        vec![item],
        dest.clone(),
        or_io::container::rpf::BodyForm::Decoded,
        false,
        OnClash::KeepBoth,
        or_job::Cancel::new(),
        or_job::Progress::new(),
    );
    assert_eq!(report.counts.failed, 1);
    assert_eq!(std::fs::read_dir(dest).unwrap().count(), 0);
}

#[test]
fn a_later_chunk_failure_preserves_the_destination_and_removes_partial_output() {
    use crate::state::extract_job::{run_extraction, ExtractItem, OnClash};
    use or_io::container::rpf::{BodyForm, Rpf9Builder, Rpf9Toc};

    let first = b"table only bytes";
    let mut bytes = Rpf9Builder::serialize(
        &Rpf9Toc {
            bytes: or_test_support::rpf9::chunked_table(11, &[(16, 16, false), (16, 16, true)]),
            key_id: 0,
            registry_id: 0,
            entry_count: 0,
            groups: Vec::new(),
        },
        &keys(),
    )
    .unwrap();
    bytes.resize(0x1000, 0x78);
    bytes.extend(first);
    bytes.extend([0; 16]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken.rpf");
    std::fs::write(&path, bytes).unwrap();
    let table = Rpf9TableDocument::open_using(&path, || Ok(keys()))
        .unwrap()
        .unwrap();
    table
        .tree()
        .nodes
        .get_mut(&0)
        .unwrap()
        .mount
        .archive
        .entries[0]
        .name = Some("texture.ytd".into());
    let source = table.entry(0, 0).unwrap();
    let mut partial = Vec::new();
    assert!(source.extract(&mut partial).is_err());
    assert_eq!(
        partial, first,
        "the decoder fails after writing the first chunk"
    );

    let dest = dir.path().join("out");
    std::fs::create_dir(&dest).unwrap();
    let existing = dest.join(&source.name);
    std::fs::write(&existing, b"original texture").unwrap();
    for fixes in [false, true] {
        let report = run_extraction(
            vec![ExtractItem::rpf9(source.clone())],
            dest.clone(),
            BodyForm::Decoded,
            fixes,
            OnClash::Replace,
            or_job::Cancel::new(),
            or_job::Progress::new(),
        );
        assert_eq!(report.counts.failed, 1);
        assert_eq!(report.counts.succeeded, 0);
        assert_eq!(std::fs::read(&existing).unwrap(), b"original texture");
        let remaining: Vec<_> = std::fs::read_dir(&dest)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(
            remaining.as_slice(),
            std::slice::from_ref(&existing),
            "partial output must be removed"
        );
    }
}

#[test]
fn a_name_cannot_escape_the_destination_or_alias_another_entry() {
    let (_dir, table) = opened();
    for name in [
        "../../evil.txt",
        "C:\\outside\\evil.txt",
        "/absolute/evil.txt",
        "..",
        "CON",
        "x:stream",
        "end.",
    ] {
        table
            .tree()
            .nodes
            .get_mut(&0)
            .unwrap()
            .mount
            .archive
            .entries[1]
            .name = Some(name.into());
        let source = table.entry(0, 1).unwrap();
        assert_eq!(Path::new(&source.name).components().count(), 1);
        assert!(!source.name.contains(['/', '\\', ':']));
        assert!(!["..", "CON", "end."].contains(&source.name.as_str()));
        assert_eq!(address(&source.path).unwrap().1, [1]);
        let doc = Document::new(
            1,
            Some(source.path),
            DocPayload::Hex(super::super::super::HexDocument {
                bytes: vec![],
                banner: String::new(),
            }),
        );
        assert_eq!(doc.read_only, Some(READ_ONLY));
    }
    assert!(address(&table.path.join(MARKER).join("bad").join("x")).is_err());
}

#[test]
fn unsupported_entry_opening_does_not_start_a_scan() {
    let _serial = crate::ui::theme::serial();
    let (_dir, table) = opened();
    table
        .tree()
        .nodes
        .get_mut(&0)
        .unwrap()
        .mount
        .archive
        .entries[1]
        .class = Some(14);
    let mut state = AppState::from_prefs(Prefs::default());
    state.open_rpf9_entry(&table, 0, 1);
    assert!(state.load_receiver.is_none());
    assert!(state.documents.is_empty());
    assert!(state.show_error_dialog);
    assert!(state
        .error_dialog_message
        .contains("unsupported entry layout"));
}

#[test]
fn the_open_button_loads_the_selected_table_entry() {
    use crate::state::nothing_draws_over_what_it_belongs_to::{painted, Panel};
    let _serial = crate::ui::theme::serial();
    let (_dir, table) = opened();
    table.tree().selected = Some((0, 1));
    let mut state = AppState::from_prefs(Prefs::default());
    state.insert_document(
        table.path.clone(),
        DocPayload::Rpf9Table(table.clone()),
        None,
        None,
    );
    let mut panel = Panel::new(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(800.0, 600.0),
    ));
    panel.frame(Vec::new(), |ui| {
        crate::ui::panels::inspector::draw_body(ui, &mut state)
    });
    let before = panel.frame(Vec::new(), |ui| {
        crate::ui::panels::inspector::draw_body(ui, &mut state)
    });
    assert!(
        state.load_receiver.is_none(),
        "drawing alone must not read an entry"
    );
    let at = painted(&before)
        .iter()
        .find(|p| p.text == "Open")
        .expect("open button")
        .rect
        .center();
    panel.frame(vec![egui::Event::PointerMoved(at)], |ui| {
        crate::ui::panels::inspector::draw_body(ui, &mut state)
    });
    for pressed in [true, false] {
        panel.frame(
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
            |ui| crate::ui::panels::inspector::draw_body(ui, &mut state),
        );
    }
    assert!(
        state.load_receiver.is_some(),
        "clicking must start the entry read"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while state.load_receiver.is_some() {
        state.poll_load_result();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(matches!(
        state.active_document().unwrap().payload,
        DocPayload::Hex(_)
    ));
    panel.frame(Vec::new(), |ui| {
        assert!(
            crate::ui::panels::inspector::draw_archive_table(ui, &mut state),
            "the parent tree stays available while its file is open"
        );
    });
}

#[test]
fn text_opens_as_text_and_a_gta_vi_resource_is_not_given_a_gta_v_reader() {
    let (_dir, table) = opened();
    table
        .tree()
        .nodes
        .get_mut(&0)
        .unwrap()
        .mount
        .archive
        .entries[1]
        .name = Some("info.txt".into());
    assert!(matches!(
        table.entry(0, 1).unwrap().payload().unwrap(),
        DocPayload::Text(_)
    ));
    table
        .tree()
        .nodes
        .get_mut(&0)
        .unwrap()
        .mount
        .archive
        .entries[1]
        .name = Some("model.ydr".into());
    assert!(matches!(
        table.entry(0, 1).unwrap().payload().unwrap(),
        DocPayload::Hex(_)
    ));
}

#[test]
fn a_twenty_byte_nested_entry_opens_reloads_and_extracts_from_its_table() {
    use or_io::container::rpf::{Rpf9Builder, Rpf9Toc};
    let _serial = crate::ui::theme::serial();
    let encode = |bytes| {
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
        .unwrap()
    };
    let mut bytes = encode(or_test_support::rpf9::table(&[(1, 0x1000, 0x1020, 0)]));
    bytes.resize(0x1000, 0x78);
    bytes.extend(encode(or_test_support::rpf9::resource_table(&[(
        16, 16, false,
    )])));
    bytes.resize(0x2010, 0x78);
    let expected = b"table only bytes";
    bytes.extend(expected);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("resource.rpf");
    std::fs::write(&path, bytes).unwrap();
    let table = Rpf9TableDocument::open_using(&path, || Ok(keys()))
        .unwrap()
        .unwrap();
    table.toggle(0, 0);
    wait(&table);
    let source = table.entry(0x1000, 0).unwrap();
    let (_, chain) = address(&source.path).unwrap();
    assert_eq!(chain, [0, 0]);
    let reloaded = Rpf9TableDocument::open_using(&path, || Ok(keys()))
        .unwrap()
        .unwrap();
    assert_eq!(
        read_chain(&reloaded, &chain).unwrap().bytes().unwrap(),
        expected
    );
    let mut state = AppState::from_prefs(Prefs::default());
    state.open_rpf9_entry(&table, 0x1000, 0);
    let dest = dir.path().join("out");
    assert!(state.extract_rpf9_entry(&table, 0x1000, 0, dest.clone()));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while state.load_receiver.is_some() || state.extract_jobs.is_running() {
        state.poll_load_result();
        state.poll_extract_job();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let doc = state.active_document().unwrap();
    assert_eq!(doc.read_only, Some(READ_ONLY));
    let DocPayload::Hex(hex) = &doc.payload else {
        panic!("expected a byte view");
    };
    assert_eq!(hex.bytes, expected);
    assert_eq!(std::fs::read(dest.join(&source.name)).unwrap(), expected);
}

mod install {
    use super::*;
    use crate::state::extract_job::{run_extraction, ExtractItem, OnClash};

    #[test]
    fn model_and_map_table_previews_open_in_memory_with_extensions() {
        use or_io::gta6::headerless::Resource;
        let Some(root) = std::env::var_os("OPENRAGE_GTA6_BUILD") else {
            or_test_support::skip_install(
                "RPF9 model and map previews",
                format_args!("set OPENRAGE_GTA6_BUILD and OPENRAGE_GTA6_KEYS"),
            );
            return;
        };
        for (archive, parent, index, extension) in [
            ("data.rpf", 14, 0, "ydr"),
            ("data.rpf", 7, 0, "ydd"),
            ("data.rpf", 26, 0, "yft"),
            ("levels.rpf", 2, 1, "ybn"),
            ("levels.rpf", 19, 0, "ymap"),
            ("levels.rpf", 0, 0, "ytyp"),
        ] {
            let path = Path::new(&root).join(archive);
            let table = Rpf9TableDocument::open(&path).unwrap().unwrap();
            let base = table.expand_table(0, parent).unwrap();
            let source = table.entry(base, index).unwrap();
            let raw_name = source.entry.display_name();
            let hash = source.entry.name64;
            let payload = source
                .payload_limited(1 << 20)
                .unwrap_or_else(|error| panic!("{archive}:{parent}/{index}: {error}"));
            match (&payload, extension) {
                (DocPayload::Gta6Drawable(resource), "ydr") => {
                    assert!(matches!(&**resource, Resource::Drawable(_)));
                }
                (DocPayload::Gta6Drawable(resource), "ydd") => {
                    assert!(matches!(&**resource, Resource::Dictionary(_)));
                }
                (DocPayload::Gta6Drawable(resource), "yft") => {
                    assert!(matches!(&**resource, Resource::DrawableWithBound(_)));
                }
                (DocPayload::Gta6Drawable(resource), "ybn") => {
                    assert!(matches!(&**resource, Resource::Bound(_)));
                }
                (DocPayload::Meta(_), "ymap" | "ytyp") => {
                    let map = crate::state::gta6_map::opened_map(&source.path)
                        .expect("the Map preview is available")
                        .expect("the map reader succeeded");
                    assert!(matches!(
                        (&*map, extension),
                        (or_io::gta6::map::MapBody::Data(_), "ymap")
                            | (or_io::gta6::map::MapBody::Types(_), "ytyp")
                    ));
                }
                _ => panic!("{archive}:{parent}/{index} has no {extension} preview"),
            }
            assert_eq!(source.path.extension().unwrap(), extension);
            assert_eq!(address(&source.path).unwrap().1, [parent, index]);
            let current = table.entry(base, index).unwrap();
            assert_eq!(current.entry.name64, hash);
            assert_eq!(current.entry.display_name(), raw_name);
            assert_eq!(current.path, source.path);
            assert!(!source.path.exists());
        }
    }

    fn walk(table: &Rpf9TableDocument, base: u64, output: &Path) -> usize {
        let entries = table.tree().nodes[&base].mount.archive.entries.clone();
        let mut count = 0;
        for entry in entries {
            let source = table
                .entry(base, entry.index)
                .unwrap_or_else(|e| panic!("{base:#x}/{}: {e}", entry.index));
            let nested = entry.kind() == Some(Gta6Class::NestedArchive);
            let expected = if nested {
                None
            } else {
                Some(
                    match source
                        .payload()
                        .unwrap_or_else(|e| panic!("open {}: {e}", source.path.display()))
                    {
                        DocPayload::Hex(hex) => hex.bytes,
                        DocPayload::Text(_)
                        | DocPayload::Meta(_)
                        | DocPayload::Gta6Drawable(_)
                        | DocPayload::Resource(or_io::Asset::TextureDict(_)) => {
                            source.bytes().unwrap()
                        }
                        _ => panic!("unexpected entry document"),
                    },
                )
            };
            let dest = output.join(format!("{base:x}_{}", entry.index));
            let path = dest.join(&source.name);
            let item = ExtractItem::rpf9(source.clone());
            let report = run_extraction(
                vec![item],
                dest,
                or_io::container::rpf::BodyForm::Decoded,
                false,
                OnClash::KeepBoth,
                or_job::Cancel::new(),
                or_job::Progress::new(),
            );
            assert_eq!(report.counts.failed, 0, "{:?}", report.failures);
            assert_eq!(report.counts.succeeded, 1);
            if let Some(expected) = expected {
                assert_eq!(std::fs::read(&path).unwrap(), expected);
            } else {
                use std::io::{Read, Seek, SeekFrom};
                let mut input = File::open(&table.path).unwrap();
                input
                    .seek(SeekFrom::Start(base + entry.start.unwrap()))
                    .unwrap();
                let mut output = File::open(&path).unwrap();
                let mut remaining = source.read.decoded_len();
                assert_eq!(output.metadata().unwrap().len(), remaining);
                let (mut before, mut after) = ([0; 65536], [0; 65536]);
                while remaining > 0 {
                    let n = remaining.min(before.len() as u64) as usize;
                    input.read_exact(&mut before[..n]).unwrap();
                    output.read_exact(&mut after[..n]).unwrap();
                    assert_eq!(&before[..n], &after[..n]);
                    remaining -= n as u64;
                }
            }
            std::fs::remove_file(path).unwrap();
            count += 1;
            if entry.kind() == Some(Gta6Class::NestedArchive) {
                let child = child_extent(&table.tree().nodes[&base], entry.index).unwrap();
                table.toggle(base, entry.index);
                wait(table);
                assert!(table.tree().errors.is_empty());
                count += walk(table, child.start, output);
            }
        }
        count
    }

    #[test]
    fn audio_entries_show_awc_without_opening_or_extracting_their_bodies() {
        let Some(root) = std::env::var_os("OPENRAGE_GTA6_BUILD") else {
            or_test_support::skip_install(
                "RPF9 audio types",
                format_args!("set OPENRAGE_GTA6_BUILD and OPENRAGE_GTA6_KEYS"),
            );
            return;
        };
        let path = Path::new(&root).join("audio.rpf");
        let table = Arc::new(Rpf9TableDocument::open(&path).unwrap().unwrap());
        let parent = table.tree().nodes[&0]
            .mount
            .archive
            .entries
            .iter()
            .find(|entry| entry.display_name() == "hash_0b5060262bec3202")
            .unwrap()
            .index;
        let base = table.expand_table(0, parent).unwrap();
        let entries = table.tree().nodes[&base].mount.archive.entries.clone();
        let selected = entries
            .iter()
            .find(|entry| entry.display_name() == "hash_0fc08147e594a40c")
            .unwrap();
        assert_eq!(selected.display_name(), "hash_0fc08147e594a40c");
        for entry in &entries {
            let (kind, detected) = table.tree().entry_type(base, entry.index);
            let name = entry.display_name();
            assert_eq!(
                crate::state::files_view::rpf9_display_name(&name, kind),
                format!("{name}.awc")
            );
            assert!(
                detected,
                "{} must be identified before opening its body",
                entry.index
            );
            let node = crate::state::RpfSessionNode {
                id: 1,
                parent: None,
                name: name.clone(),
                kind: crate::state::RpfSessionNodeKind::BinaryFile,
                children: Vec::new(),
                backing: RpfNodeBacking::Rpf9File {
                    table: Arc::clone(&table),
                    base,
                    index: entry.index,
                },
            };
            assert_eq!(node.display_name(), format!("{name}.awc"));
            assert_eq!(node.rpf9_entry(), Some((entry.index, entry.name64)));
            let icon = crate::ui::icons::rpf_node_icon(&node, false, false, true, false);
            let expected = crate::ui::icons::doc_kind_icon(DocumentKind::Awc, false);
            assert_eq!(icon.glyph, expected.glyph);
            assert_eq!(icon.type_tooltip, DocumentKind::Awc.info().noun);
            let source = table.entry(base, entry.index).unwrap();
            let bytes = source.bytes().unwrap();
            let audio = or_io::awc::AwcFile::parse(&bytes, or_io::awc::AwcKey::default()).unwrap();
            assert!(!audio.streams.is_empty());
            assert_eq!(
                Gta6EntryContent::detect(entry, &source.extra, &bytes)
                    .unwrap()
                    .kind,
                kind
            );
            assert_eq!(source.name, format!("{name}.awc"));
            assert_eq!(node.name, name);
            assert!(!source.path.exists());
        }
        println!(
            "{} audio entries identified before opening and cross-checked with the AWC reader",
            entries.len()
        );
    }

    #[test]
    fn encrypted_psin_is_named_before_its_body_is_opened() {
        let Some(root) = std::env::var_os("OPENRAGE_GTA6_BUILD") else {
            or_test_support::skip_install(
                "RPF9 PSIN types",
                format_args!("set OPENRAGE_GTA6_BUILD and OPENRAGE_GTA6_KEYS"),
            );
            return;
        };
        let path = Path::new(&root).join("appdata0.rpf");
        let table = Arc::new(Rpf9TableDocument::open(&path).unwrap().unwrap());
        let entry = table.tree().nodes[&0]
            .mount
            .archive
            .entries
            .iter()
            .find(|entry| entry.display_name() == "hash_336373b5566693f1")
            .unwrap()
            .clone();
        assert!(entry.cipher.is_some());
        assert!(entry.stored < entry.real);
        let (kind, detected) = table.tree().entry_type(0, entry.index);
        assert!(detected);
        assert_eq!(
            crate::state::files_view::rpf9_display_name(&entry.display_name(), kind),
            "hash_336373b5566693f1.psin"
        );
        let node = crate::state::RpfSessionNode {
            id: 1,
            parent: None,
            name: entry.display_name(),
            kind: crate::state::RpfSessionNodeKind::BinaryFile,
            children: Vec::new(),
            backing: RpfNodeBacking::Rpf9File {
                table: Arc::clone(&table),
                base: 0,
                index: entry.index,
            },
        };
        let icon = crate::ui::icons::rpf_node_icon(&node, false, false, true, false);
        assert_eq!(
            icon.glyph,
            crate::ui::icons::doc_kind_icon(DocumentKind::Meta, false).glyph
        );
        let source = table.entry(0, entry.index).unwrap();
        let bytes = source.bytes().unwrap();
        assert_eq!(
            or_io::meta::binary::detect_binary_format(&bytes),
            Some(or_io::meta::binary::BinaryFormat::Pso)
        );
        assert_eq!(
            Gta6EntryContent::detect(&entry, &source.extra, &bytes)
                .unwrap()
                .kind,
            kind
        );
        let DocPayload::Meta(metadata) = source.payload().unwrap() else {
            panic!("the ciphered PSIN entry opens in its metadata preview")
        };
        assert!(metadata.parse_error.is_none());
        assert!(metadata.parsed.is_some());
        let mut extracted = Vec::new();
        source.extract(&mut extracted).unwrap();
        assert_eq!(extracted, bytes);
        assert_eq!(source.name, "hash_336373b5566693f1.psin");
        assert!(!source.path.exists());
    }

    #[test]
    fn supported_media_metadata_and_navigation_open_from_real_tables_in_memory() {
        let Some(root) = std::env::var_os("OPENRAGE_GTA6_BUILD") else {
            or_test_support::skip_install(
                "RPF9 specialised previews",
                format_args!("set OPENRAGE_GTA6_BUILD and OPENRAGE_GTA6_KEYS"),
            );
            return;
        };
        let root = Path::new(&root);
        type Case = (&'static str, &'static [(&'static [u32], DocumentKind)]);
        let cases: &[Case] = &[
            (
                "audio.rpf",
                &[
                    (&[102, 5], DocumentKind::Awc),
                    (&[102, 62], DocumentKind::Awc),
                    (&[14, 0], DocumentKind::Gta6AudioData),
                    (&[14, 2], DocumentKind::Gta6AudioData),
                    (&[14, 4], DocumentKind::Gta6AudioData),
                ],
            ),
            (
                "navmeshes.rpf",
                &[(&[0, 0], DocumentKind::Ynv), (&[0, 1], DocumentKind::Ynv)],
            ),
            (
                "data.rpf",
                &[
                    (&[8, 0], DocumentKind::Meta),
                    (&[8, 1], DocumentKind::Meta),
                    (&[29, 0], DocumentKind::Ypt),
                    (&[29, 1], DocumentKind::Ypt),
                ],
            ),
            (
                "anim.rpf",
                &[
                    (&[5, 0], DocumentKind::Gta6Anim),
                    (&[5, 1], DocumentKind::Gta6Anim),
                    (&[5, 2], DocumentKind::Gta6Anim),
                    (&[111, 0], DocumentKind::Meta),
                ],
            ),
            (
                "appdata0.rpf",
                &[(&[0], DocumentKind::Meta), (&[1], DocumentKind::Meta)],
            ),
            (
                "movies.rpf",
                &[(&[0], DocumentKind::Bik), (&[1], DocumentKind::Bik)],
            ),
        ];
        let mut count = 0;
        let mut streams = 0;
        for (name, entries) in cases {
            let table = Rpf9TableDocument::open(&root.join(name)).unwrap().unwrap();
            for (chain, kind) in *entries {
                let source = read_chain(&table, chain).unwrap();
                let payload = source
                    .payload()
                    .unwrap_or_else(|e| panic!("{name}:{chain:?}: {e}"));
                let doc = Document::new(1, Some(source.path.clone()), payload);
                assert_eq!(doc.kind, *kind, "{name}:{chain:?}");
                assert_eq!(doc.read_only, Some(READ_ONLY));
                assert!(doc.is_from_rpf9_table());
                match doc.payload {
                    DocPayload::Meta(meta) => {
                        assert!(!meta.parsed.unwrap().instance.fields.is_empty())
                    }
                    DocPayload::Resource(or_io::Asset::NavMesh(mesh)) => {
                        assert!(mesh.polys_array.is_some())
                    }
                    DocPayload::Awc(mut audio) => {
                        let key = or_io::awc::AwcKey::default();
                        let file = audio.file(key).unwrap();
                        for row in or_media::awc::streams(file) {
                            let stream = or_media::awc::open(file, key, row.stream).unwrap();
                            let samples = stream
                                .audio(&or_media::Decoders::new())
                                .unwrap()
                                .read_all_i16()
                                .unwrap();
                            assert!(!samples.is_empty());
                            streams += 1;
                        }
                    }
                    _ => {}
                }
                assert!(!source.path.exists());
                count += 1;
            }
        }
        assert_eq!(count, 19);
        assert_eq!(streams, 106);
    }

    #[test]
    fn every_entry_in_the_selected_archive_opens_and_extracts_in_memory() {
        let Some(archive) = std::env::var_os("OPENRAGE_GTA6_ARCHIVE") else {
            or_test_support::skip_install(
                "RPF9 table opening and in-memory extraction",
                format_args!("set OPENRAGE_GTA6_ARCHIVE and OPENRAGE_GTA6_KEYS"),
            );
            return;
        };
        let table = Rpf9TableDocument::open(Path::new(&archive))
            .unwrap()
            .unwrap();
        let mut pending = vec![0];
        let mut visited = std::collections::BTreeSet::new();
        let (mut entries, mut leaves, mut extracted_bytes) = (0, 0, 0u64);
        let mut failures = Vec::new();
        while let Some(base) = pending.pop() {
            if !visited.insert(base) {
                continue;
            }
            let rows = table.tree().nodes[&base].mount.archive.entries.clone();
            for entry in rows {
                entries += 1;
                let nested = entry.kind() == Some(Gta6Class::NestedArchive);
                let result = (|| -> Result<(), String> {
                    let source = table.entry(base, entry.index)?;
                    let expected = source.decoded_len();
                    let written = if nested {
                        source.extract(&mut std::io::sink())?
                    } else {
                        if expected > LIMIT {
                            return Err(format!(
                                "decoded size {expected} exceeds preview limit {LIMIT}"
                            ));
                        }
                        let mut bytes = Vec::new();
                        let written = source.extract(&mut bytes)?;
                        if bytes.len() as u64 != expected {
                            return Err(format!(
                                "decoded length {} differs from {expected}",
                                bytes.len()
                            ));
                        }
                        source.payload_from_bytes_at(&source.path, bytes)?;
                        leaves += 1;
                        written
                    };
                    if written != expected {
                        return Err(format!(
                            "extracted length {written} differs from {expected}"
                        ));
                    }
                    extracted_bytes += written;
                    Ok(())
                })();
                if let Err(error) = result {
                    failures.push(format!("table {base:#x}, entry {}: {error}", entry.index));
                }
                if nested {
                    match table.expand_table(base, entry.index) {
                        Ok(child) => pending.push(child),
                        Err(error) => failures.push(format!(
                            "expand table {base:#x}, entry {}: {error}",
                            entry.index
                        )),
                    }
                }
            }
        }
        table.check_current().unwrap();
        assert!(entries > 0, "the selected archive is empty");
        assert!(
            failures.is_empty(),
            "{} failures: {:#?}",
            failures.len(),
            &failures[..failures.len().min(20)]
        );
        assert!(leaves > 0, "the selected archive has no leaf entries");
        println!(
            "{entries} entries extracted in memory ({extracted_bytes} bytes), {leaves} leaf payloads opened, {} tables",
            visited.len()
        );
    }

    #[test]
    fn every_entry_in_the_selected_archive_opens_and_extracts_from_tables() {
        let Some(archive) = std::env::var_os("OPENRAGE_GTA6_ARCHIVE") else {
            or_test_support::skip_install(
                "RPF9 table opening and extraction",
                format_args!("set OPENRAGE_GTA6_ARCHIVE and OPENRAGE_GTA6_KEYS"),
            );
            return;
        };
        let snapshot = std::env::var_os("OPENRAGE_GTA6_KEYS").expect("OPENRAGE_GTA6_KEYS");
        let keys = GtaViKeys::from_file(Path::new(&snapshot)).expect("load the key snapshot");
        let table = Rpf9TableDocument::open_using(Path::new(&archive), || Ok(keys))
            .unwrap()
            .unwrap();
        let output = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let count = walk(&table, 0, output.path());
        assert!(count > 0, "the selected archive is empty");
        println!(
            "{count} entries opened, extracted and compared through the application; {} tables",
            table.tree().nodes.len()
        );
    }
}

#[path = "rpf9_entry_media_tests.rs"]
mod media;
