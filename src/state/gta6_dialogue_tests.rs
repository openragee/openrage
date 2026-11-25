use super::*;
use or_io::gta6::text::{TextEntry, TextItem};

/// A conversation body that states `labels` under `key`. The app's lines are one set, so each
/// test that adds to them states a key no other names.
fn body(key: &str, labels: &[&str]) -> Vec<u8> {
    let mut out = vec![0];
    for label in labels {
        out.extend(key.as_bytes());
        out.push(0);
        out.extend(label.as_bytes());
        out.extend([0, 0]);
    }
    out
}

/// A one-group table of `texts`, each under its label's hash.
fn table(texts: &[(u32, &str)]) -> TextTable {
    let entries = (texts.iter().enumerate())
        .map(|(slot, (key, text))| TextEntry {
            key: *key,
            group: 0,
            slot: slot as u8,
            item: TextItem::Text((*text).to_string()),
        })
        .collect();
    TextTable { groups: 1, entries }
}

fn zlib(offset: u64, content: Rpf9Content, inflated: u64) -> Rpf9ScanFind {
    Rpf9ScanFind {
        offset,
        kind: Rpf9BodyKind::Zlib(Rpf9ZlibRun {
            content,
            chunks: 1,
            stored_pieces: 0,
            first_chunk_len: 8,
            first_chunk_inflated: inflated,
            inflated,
            signature: None,
            resource: None,
        }),
        span: 16,
        stored_len: Some(8),
        inflated_len: Some(inflated),
        container: None,
    }
}

#[test]
fn a_conversation_body_s_lines_name_a_text_row_and_tag_an_exported_stream() {
    note_body(&body(
        "APPTESTAU_ZZ01",
        &["APPTEST_ONE_QA_1", "APPTEST_TWO_2"],
    ));
    let line = DialogueLine {
        key: "APPTESTAU_ZZ01".into(),
        number: 1,
        label: "APPTEST_ONE_QA_1".into(),
    };
    assert_eq!(
        line_of(line.label_hash()),
        Some(("APPTESTAU_ZZ01_01".to_string(), line.label.clone()))
    );
    assert_eq!(line_of(0x0BAD_0BAD), None);

    let comment = |f: &str, v: &str| (f.to_string(), v.to_string());
    let named = [
        comment("TITLE", "APPTESTAU_ZZ01_01"),
        comment("LABEL", "APPTEST_ONE_QA_1"),
        comment("SPEAKER", "QA"),
    ];
    assert_eq!(comments(line.stream_id()), named);

    note_table(&table(&[(line.label_hash(), "~z~Bonjour,~n~toi.")]));
    let mut with_subtitle = named.to_vec();
    with_subtitle.push(comment("SUBTITLE", "Bonjour, toi."));
    assert_eq!(comments(line.stream_id()), with_subtitle);
    assert!(comments(0x0BAD_0BAD).is_empty());
}

#[test]
fn the_filter_keeps_lines_by_speaker_and_by_a_text_of_the_language_picked() {
    // Lines of their own, so no other test's filter or lines are in the way.
    let mut lines = Lines::default();
    let labels = ["FILT_GO_QA_1", "FILT_STAY_ZB_2", "FILT_SCENE_3"];
    assert_eq!(lines.dialogue.add(&body("FILTAU_AA01", &labels)), 3);
    lines.refresh();
    let hash = |label: &str| or_io::gta6::hash::name_hash32(label.as_bytes());
    let hashes = labels.map(hash);
    lines.keep(
        "northern",
        BTreeMap::from([
            (hashes[0], "Go there.".to_string()),
            (hashes[1], "Stay here.".to_string()),
        ]),
    );
    lines.keep(
        "eastern",
        BTreeMap::from([(hashes[0], "Vas-y.".to_string())]),
    );
    assert_eq!(lines.count(), 3);
    assert!(lines.holds("eastern") && !lines.holds("southern"));
    assert_eq!(lines.text(hashes[0], "eastern"), Some("Vas-y."));
    assert_eq!(lines.text(hashes[1], "eastern"), None);
    assert_eq!(lines.any_text(hashes[1]), Some("Stay here."));

    lines.filter_lines();
    assert_eq!(
        lines.shown(),
        hashes,
        "no filter keeps every line, in order"
    );
    lines.filter.speaker = " qa ".into();
    lines.filter_lines();
    assert_eq!(lines.shown(), [hashes[0]]);
    // A text is looked for in the label and in the subtitle of the language picked.
    lines.filter = Filter {
        speaker: String::new(),
        text: "HERE".into(),
        language: "northern".into(),
    };
    lines.filter_lines();
    assert_eq!(lines.shown(), [hashes[0], hashes[1]]);
    lines.filter.language = "eastern".into();
    lines.filter_lines();
    assert!(lines.shown().is_empty());
    lines.filter.text = "scene".into();
    lines.filter_lines();
    assert_eq!(lines.shown(), [hashes[2]]);
}

#[test]
fn a_language_is_its_registered_name_or_its_archive_s_place() {
    let three: Vec<String> = ["northern", "eastern", "southern"]
        .map(String::from)
        .to_vec();
    assert_eq!(language_of(&three, 1, 3), "eastern");
    // A file that holds another count of language archives is not given the names.
    assert_eq!(language_of(&three, 1, 2), "#2");
    assert_eq!(language_of(&[], 0, 3), "#1");

    let mut lines = Lines::default();
    assert!(lines.languages().is_empty());
    assert_eq!(lines.first_language(), "#1");
    lines.places = 2;
    lines.keep(OPENED, BTreeMap::new());
    lines.keep("#7", BTreeMap::new());
    assert_eq!(lines.languages(), ["#1", "#2", "#7"]);
    lines.registered = three.clone();
    assert_eq!(lines.languages(), ["northern", "eastern", "southern", "#7"]);
    assert_eq!(lines.first_language(), "northern");
}

#[test]
fn an_rbf0_body_the_strict_reader_refuses_still_opens_under_a_note() {
    // A root element of no attribute, then its close.
    let mut doc = b"RBF0".to_vec();
    doc.extend([0, 0, 1, 0, b'A', 0, 0, 0, 0, 0, 0, 0xFF, 0xFF]);
    let path = Path::new("made-up.rpf/0x1000.pso");
    let find = zlib(0x1000, Rpf9Content::Rbf, doc.len() as u64);
    opened(path, &find, &doc);
    assert_eq!(strict_note(path), None);
    opened(path, &find, &doc[..doc.len() - 2]);
    let note = strict_note(path).expect("a note");
    assert!(note.contains("lenient"), "{note}");
    // The note goes once the body reads.
    opened(path, &find, &doc);
    assert_eq!(strict_note(path), None);
}

#[test]
fn an_archive_s_rows_are_sorted_into_what_the_index_reads() {
    let path = Path::new("made-up.rpf");
    let awc = Rpf9ScanFind {
        kind: Rpf9BodyKind::Awc {
            flags: 0x25,
            streams: 3,
            chunks: 6,
        },
        ..zlib(0x3000, Rpf9Content::Pso, 8)
    };
    let rows: Vec<Row> = [
        zlib(0x1000, Rpf9Content::Pso, 8),
        zlib(0x2000, Rpf9Content::AudioData, 8),
        awc,
        zlib(0x4000, Rpf9Content::Xml, 3_000),
        // A long XML body is no change set, and a table in no language archive has no language.
        zlib(0x5000, Rpf9Content::Xml, CHANGE_SET_LIMIT + 1),
        zlib(0x6000, Rpf9Content::TextTable, 8),
    ]
    .into_iter()
    .map(|find| Row::Body(Box::new(find)))
    .chain([Row::Missing {
        from: 0x7000,
        to: 0x8000,
        bytes: 0x1000,
    }])
    .collect();
    let want = Archive {
        path: path.to_path_buf(),
        conversations: vec![0x1000],
        tables: Vec::new(),
        languages: 0,
        change_sets: vec![0x4000],
        audio_data: vec![0x2000],
        containers: vec![(0x3000, 0x25, 3)],
    };
    let read = Archive::of(path, &rows);
    assert_eq!(read, want);
    assert_eq!(read.bodies(), 4);
}

#[test]
fn a_stream_asked_for_plays_once_in_the_container_that_holds_it() {
    let row = |stream: usize, id: u32, channels: &[u32]| crate::audio::AwcStreamRow {
        stream,
        id,
        codec: or_media::Codec::Opus,
        channels: 2,
        sample_rate: 48_000,
        samples: 0,
        loop_point: None,
        opus_channels: channels.iter().map(|id| (*id, 1, 0)).collect(),
        animation: false,
    };
    // The second row's second channel is the stream, under the bits a table's word adds.
    let rows = [row(0, 0x111, &[]), row(4, 0, &[0x222, 0xE000_0333])];
    let archive = Path::new("made-up-audio.rpf");
    let container = body_path(archive, 0x9000, View::Audio);
    *asked() = Some((container.clone(), 0x333));
    assert_eq!(play_asked(Path::new("another.awc"), &rows), None);
    assert_eq!(play_asked(&container, &rows), Some(4));
    assert_eq!(play_asked(&container, &rows), None, "asked once");
}

#[test]
fn a_sound_s_track_ids_are_those_the_markers_of_one_of_its_streams_state() {
    let mut lines = Lines::default();
    lines.track_ids.insert(0x0123, vec![7, 9]);
    let sound = |stream: u32| StreamedSound {
        key: 1,
        duration_ms: 0,
        channels: vec![(2, None), (3, Some(stream))],
    };
    // A stream's id is the low 29 bits of the word a sound names it by.
    assert_eq!(lines.track_ids(&sound(0xE000_0123)), [7, 9]);
    assert!(lines.track_ids(&sound(0x0456)).is_empty());
}
