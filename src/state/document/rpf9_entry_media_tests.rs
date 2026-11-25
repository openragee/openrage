use super::*;

fn media_table_fixture(body: &[u8]) -> (tempfile::TempDir, Rpf9TableDocument) {
    let mut body = body.to_vec();
    body.resize(body.len().next_multiple_of(16), 0);
    use or_io::container::rpf::{Rpf9Builder, Rpf9Toc};
    let mut bytes = Rpf9Builder::serialize(
        &Rpf9Toc {
            bytes: or_test_support::rpf9::table(&[(0, 0x1000, body.len() as u64, 0xff)]),
            key_id: 0,
            registry_id: 0,
            entry_count: 0,
            groups: Vec::new(),
        },
        &keys(),
    )
    .unwrap();
    bytes.resize(0x1000, 0);
    bytes.extend(body);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("audio.rpf");
    std::fs::write(&path, bytes).unwrap();
    let table = Rpf9TableDocument::open_using(&path, || Ok(keys()))
        .unwrap()
        .unwrap();
    (dir, table)
}

fn table_pcm_awc() -> Vec<u8> {
    let mut record = vec![0u8; 32];
    record[..4].copy_from_slice(&2u32.to_le_bytes());
    record[4..8].copy_from_slice(&(-1i32).to_le_bytes());
    record[8..10].copy_from_slice(&48000u16.to_le_bytes());
    record[12..16].copy_from_slice(&2u32.to_le_bytes());
    record[29] = 1;
    let data = [-1234i16, 2345]
        .into_iter()
        .flat_map(i16::to_le_bytes)
        .collect::<Vec<_>>();
    let chunks = [(0x76u64, record), (0x55, data)];
    let table_len = 16 + 4 + 8 * chunks.len();
    let mut bytes = b"ADAT".to_vec();
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(0xff02u16.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend((table_len as u32).to_le_bytes());
    bytes.extend(((chunks.len() as u32) << 29 | 1).to_le_bytes());
    let mut offset = table_len;
    for (kind, data) in &chunks {
        bytes.extend(((kind << 56) | ((data.len() as u64) << 28) | offset as u64).to_le_bytes());
        offset += data.len();
    }
    for (_, data) in chunks {
        bytes.extend(data);
    }
    bytes.resize(bytes.len().next_multiple_of(16), 0);
    bytes
}

#[test]
fn awc_table_entries_open_and_decode_from_memory_despite_their_suffix() {
    let (dir, table) = media_table_fixture(&table_pcm_awc());
    table
        .tree()
        .nodes
        .get_mut(&0)
        .unwrap()
        .mount
        .archive
        .entries[0]
        .name = Some("sound.txt".into());
    let source = table.entry(0, 0).unwrap();
    assert!(!source.path.exists());
    let DocPayload::Awc(mut audio) = source.payload().unwrap() else {
        panic!("the confirmed audio header must select the audio preview");
    };
    let key = or_io::awc::AwcKey::default();
    let file = audio.file(key).unwrap();
    let rows = or_media::awc::streams(file);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].info.codec, or_media::Codec::Pcm16);
    let stream = or_media::awc::open(file, key, rows[0].stream).unwrap();
    let samples = stream
        .audio(&or_media::Decoders::new())
        .unwrap()
        .read_all_i16()
        .unwrap();
    assert_eq!(samples, [-1234, 2345]);
    assert!(!source.path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn malformed_awc_and_unsupported_endianness_are_errors_not_hex() {
    let mut malformed = table_pcm_awc();
    malformed[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut big_endian = table_pcm_awc();
    big_endian[..4].copy_from_slice(b"TADA");
    for body in [malformed, big_endian] {
        let (_dir, table) = media_table_fixture(&body);
        assert!(table.entry(0, 0).unwrap().payload().is_err());
        assert!(table.tree().errors.contains_key(&(0, 0)));
    }
}

#[test]
fn table_metadata_and_movies_select_their_content_reader() {
    use crate::state::opening_an_rpf9_archive::{kb2n, table_form_pso};
    let (_dir, table) = media_table_fixture(&table_form_pso());
    let source = table.entry(0, 0).unwrap();
    let DocPayload::Meta(meta) = source.payload().unwrap() else {
        panic!("PSIN metadata view")
    };
    assert_eq!(
        meta.parsed.unwrap().instance.fields[0].1,
        or_io::meta::Value::UInt(77)
    );
    assert!(!source.path.exists());
    let (_dir, table) = media_table_fixture(&kb2n());
    let source = table.entry(0, 0).unwrap();
    let DocPayload::Bik(movie) = source.payload().unwrap() else {
        panic!("Bink movie view")
    };
    assert!(movie.header.is_some());
    assert!(!source.path.exists());
}

#[test]
fn a_malformed_confirmed_metadata_body_is_reported_as_an_error() {
    let mut bytes = crate::state::opening_an_rpf9_archive::table_form_pso();
    bytes.truncate(16);
    let (_dir, table) = media_table_fixture(&bytes);
    assert!(table.entry(0, 0).unwrap().payload().is_err());
    assert!(table.tree().errors.contains_key(&(0, 0)));
}

#[test]
fn table_bodies_keep_metadata_text_and_unknown_binary_distinct() {
    let pso = crate::state::opening_an_rpf9_archive::table_form_pso();
    let mut xml = br#"<?xml version="1.0"?><root/>"#.to_vec();
    xml.resize(xml.len().next_multiple_of(16), b' ');
    let mut binary = vec![0x90, 0, 0x11, 0x22];
    binary.resize(48, 0);
    for (body, kind) in [
        (pso, DocumentKind::Meta),
        (xml, DocumentKind::Text),
        (binary, DocumentKind::Hex),
    ] {
        let (dir, table) = media_table_fixture(&body);
        let source = table.entry(0, 0).unwrap();
        let payload = source.payload().unwrap();
        if kind == DocumentKind::Text {
            assert!(matches!(payload, DocPayload::Text(_) | DocPayload::Meta(_)));
        } else {
            assert_eq!(payload.kind(), kind);
        }
        if let DocPayload::Hex(hex) = payload {
            assert_eq!(hex.bytes, source.bytes().unwrap());
        }
        let mut extracted = Vec::new();
        source.extract(&mut extracted).unwrap();
        assert_eq!(extracted, source.bytes().unwrap());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(!source.path.exists());
    }
}

#[test]
fn a_compressed_table_metadata_body_uses_its_exact_chunks_without_resource_guessing() {
    use or_io::container::rpf::{Rpf9Builder, Rpf9Toc};

    let plain = crate::state::opening_an_rpf9_archive::table_form_pso();
    let stored = crate::state::opening_an_rpf9_archive::zlib(&plain);
    let mut bytes = Rpf9Builder::serialize(
        &Rpf9Toc {
            bytes: or_test_support::rpf9::chunked_table(
                0xff,
                &[(plain.len() as u32, stored.len() as u32, true)],
            ),
            key_id: 0,
            registry_id: 0,
            entry_count: 0,
            groups: Vec::new(),
        },
        &keys(),
    )
    .unwrap();
    bytes.resize(0x1000, 0);
    bytes.extend(&stored);
    bytes.resize(bytes.len().next_multiple_of(16), 0);
    // Bytes beyond the table's stored extent cannot lengthen the decoded entry.
    bytes.extend([0xa5; 128]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("metadata.rpf");
    std::fs::write(&path, bytes).unwrap();
    let table = Rpf9TableDocument::open_using(&path, || Ok(keys()))
        .unwrap()
        .unwrap();
    let source = table.entry(0, 0).unwrap();
    let DocPayload::Meta(meta) = source.payload().unwrap() else {
        panic!("the decoded PSIN opens through the metadata reader")
    };
    assert_eq!(
        meta.parsed.unwrap().instance.fields[0].1,
        or_io::meta::Value::UInt(77)
    );
    let mut extracted = Vec::new();
    source.extract(&mut extracted).unwrap();
    assert_eq!(extracted, plain);
    assert_eq!(table.tree().nodes.len(), 1);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(!source.path.exists());
}

#[test]
fn a_table_script_opens_as_its_instruction_listing_and_exports_its_original_bytes() {
    let mut program = super::super::script_view::tests::program();
    program.resize(program.len().next_multiple_of(16), 0);
    let (dir, table) = media_table_fixture(&program);
    let source = table.entry(0, 0).unwrap();
    let payload = source.payload().unwrap();
    let document = Document::new(1, Some(source.path.clone()), payload);
    assert_eq!(document.read_only, Some(READ_ONLY));
    let DocPayload::Text(text) = document.payload else {
        panic!("the validated program opens as an instruction listing")
    };
    assert!(text.text.to_string().contains("000000  090  return"));
    let mut extracted = Vec::new();
    source.extract(&mut extracted).unwrap();
    assert_eq!(extracted, program);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(!source.path.exists());
}

#[test]
fn an_invalid_table_script_keeps_its_bytes_instead_of_inventing_a_listing() {
    let mut program = super::super::script_view::tests::program();
    program[0x24] ^= 1;
    program.resize(program.len().next_multiple_of(16), 0);
    let (_dir, table) = media_table_fixture(&program);
    let source = table.entry(0, 0).unwrap();
    let DocPayload::Hex(hex) = source.payload().unwrap() else {
        panic!("a mismatched script check copy has no instruction listing")
    };
    assert_eq!(hex.bytes, program);
    assert!(!source.path.exists());
}
