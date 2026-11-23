use super::*;
use crate::gta6::audio_data::STAMP;

/// A body built item by item: an item's reference fields name a target's key, its wave fields a
/// bank and a wave.
struct Body {
    kind: AudioDataKind,
    data: Vec<u8>,
    index: Vec<(u32, u32)>,
    references: Vec<(u32, u32)>,
    reference_fields: Vec<u32>,
    wave_fields: Vec<u32>,
}

impl Body {
    fn new(kind: AudioDataKind) -> Self {
        Self {
            kind,
            data: STAMP.to_le_bytes().to_vec(),
            index: Vec::new(),
            references: Vec::new(),
            reference_fields: Vec::new(),
            wave_fields: Vec::new(),
        }
    }

    /// Adds `bytes` as the item of `key`, with a reference row to each `(offset, key)` of `rows`
    /// written at its offset, in ascending offsets, and a wave field at each offset of `waves`.
    fn item(&mut self, key: u32, mut bytes: Vec<u8>, rows: &[(usize, u32)], waves: &[usize]) {
        let start = self.data.len() as u32;
        for &(at, target) in rows {
            let row = match self.references.iter().position(|r| r.0 == target) {
                Some(row) => row,
                None => {
                    self.references.push((target, 2));
                    self.references.len() - 1
                }
            };
            bytes[at..at + 4].copy_from_slice(&(row as u32).to_le_bytes());
            self.reference_fields.push(start + at as u32 + 8);
        }
        self.wave_fields
            .extend(waves.iter().map(|at| start + *at as u32 + 8));
        self.index.push((key, start));
        self.data.extend(bytes);
    }

    fn done(self) -> AudioData {
        AudioData {
            schema: 0,
            kind: self.kind,
            data: self.data,
            groups: Vec::new(),
            group_hashes: Vec::new(),
            names: 0,
            name_bytes: 0,
            index: self.index,
            index_y: Vec::new(),
            hash_fields: Vec::new(),
            wave_fields: self.wave_fields,
            group_fields: Vec::new(),
            references: self.references,
            reference_fields: self.reference_fields,
        }
    }
}

fn put(bytes: &mut [u8], at: usize, word: u32) {
    bytes[at..at + 4].copy_from_slice(&word.to_le_bytes());
}

const STATION: u32 = 0x5747_0001;
const BAND: u32 = 7_000;
const LISTS: [u32; 2] = [0x1157_0001, 0x1157_0002];
const TRACKS: [u32; 3] = [0x7AC0_0001, 0x7AC0_0002, 0x7AC0_0003];
const SOUNDS: [u32; 3] = [0x50D0_0001, 0x50D0_0002, 0x50D0_0003];

/// A station of `lists` track lists, then `pairs` 8-byte entries and `extras` 12-byte entries.
fn station_bytes(lists: usize, pairs: usize, extras: usize) -> Vec<u8> {
    let mut bytes = vec![0x11; 64 + 12 * lists + 4 + 8 * pairs + 4 + 12 * extras];
    bytes[0] = STATION_CLASS;
    put(&mut bytes, 8, STATION);
    put(&mut bytes, 12, BAND);
    put(&mut bytes, 60, lists as u32);
    put(&mut bytes, 64 + 12 * lists, pairs as u32);
    put(&mut bytes, 64 + 12 * lists + 4 + 8 * pairs, extras as u32);
    bytes
}

fn list_bytes(tracks: usize) -> Vec<u8> {
    let mut bytes = vec![0x22; 76 + 4 * tracks];
    bytes[0] = TRACK_LIST_CLASS;
    put(&mut bytes, 72, tracks as u32);
    bytes
}

fn track_bytes() -> Vec<u8> {
    let mut bytes = vec![0x33; 20];
    bytes[0] = TRACK_CLASS;
    put(&mut bytes, 16, u32::MAX);
    bytes
}

/// A music body: one station of two lists, the first of two tracks and the second of one.
fn music() -> Body {
    let mut body = Body::new(AudioDataKind::Music);
    for (track, sound) in TRACKS.iter().zip(SOUNDS) {
        body.item(*track, track_bytes(), &[(12, sound)], &[]);
    }
    body.item(
        LISTS[0],
        list_bytes(2),
        &[(76, TRACKS[0]), (80, TRACKS[1])],
        &[],
    );
    body.item(LISTS[1], list_bytes(1), &[(76, TRACKS[2])], &[]);
    let rows = [(64, LISTS[0]), (76, LISTS[1])];
    body.item(STATION, station_bytes(2, 1, 1), &rows, &[]);
    body
}

fn track(key: u32, sound: u32) -> Track {
    Track {
        key,
        sound: Some(sound),
        text: None,
    }
}

#[test]
fn a_station_lists_its_track_lists_and_their_tracks_in_order() {
    // A list's flags are the word after its class byte, which the fixture fills.
    let list = |key: u32, tracks: Vec<Track>| TrackList {
        key,
        flags: Some(0x2222_2222),
        category: None,
        tracks: Some(tracks),
    };
    let want = Station {
        key: STATION,
        flags: 0x1111_1111,
        band: BAND,
        lists: vec![
            list(
                LISTS[0],
                vec![track(TRACKS[0], SOUNDS[0]), track(TRACKS[1], SOUNDS[1])],
            ),
            list(LISTS[1], vec![track(TRACKS[2], SOUNDS[2])]),
        ],
    };
    assert_eq!(stations(&music().done()), Ok(vec![want]));
}

const ROWS: u32 = 0x2045_0001;
const CATEGORY: u32 = 0xCA7E_0001;
const TEXT: u32 = 0x7E87_0001;
const ARTISTS: [u32; 2] = [0xA271_0001, 0xA271_0002];

/// An item of `rows` rows.
fn rows_bytes(rows: usize) -> Vec<u8> {
    let mut bytes = vec![0x44; 12 + 4 * rows];
    bytes[0] = ROWS_CLASS;
    put(&mut bytes, 8, rows as u32);
    bytes
}

#[test]
fn a_list_names_its_category_and_the_tracks_of_an_item_of_rows_after_its_own() {
    let mut body = Body::new(AudioDataKind::Music);
    for (track, sound) in TRACKS.iter().zip(SOUNDS) {
        body.item(*track, track_bytes(), &[(12, sound)], &[]);
    }
    body.item(
        ROWS,
        rows_bytes(2),
        &[(12, TRACKS[1]), (16, TRACKS[2])],
        &[],
    );
    let rows = [(8, CATEGORY), (68, ROWS), (76, TRACKS[0])];
    body.item(LISTS[0], list_bytes(1), &rows, &[]);
    body.item(STATION, station_bytes(1, 0, 0), &[(64, LISTS[0])], &[]);
    let read = stations(&body.done()).expect("read");
    let list = &read[0].lists[0];
    assert_eq!(list.category, Some(CATEGORY));
    let keys: Vec<u32> = list.tracks.iter().flatten().map(|t| t.key).collect();
    assert_eq!(keys, TRACKS);

    // The item a list names for its rows is one of rows, and its rows are reference rows.
    let mut body = Body::new(AudioDataKind::Music);
    body.item(ROWS, track_bytes(), &[(12, SOUNDS[0])], &[]);
    body.item(LISTS[0], list_bytes(0), &[(68, ROWS)], &[]);
    body.item(STATION, station_bytes(1, 0, 0), &[(64, LISTS[0])], &[]);
    let why = stations(&body.done()).expect_err("refused").to_string();
    assert!(why.contains("where class 2 is named"), "{why}");

    let mut body = Body::new(AudioDataKind::Music);
    body.item(ROWS, rows_bytes(2), &[(12, TRACKS[1])], &[]);
    body.item(LISTS[0], list_bytes(0), &[(68, ROWS)], &[]);
    body.item(STATION, station_bytes(1, 0, 0), &[(64, LISTS[0])], &[]);
    let why = stations(&body.done()).expect_err("refused").to_string();
    assert!(why.contains("no reference row at +16"), "{why}");
}

/// A track text of `title` and `artists` entries, and an artist of `name`.
fn text_bytes(title: &str, artists: usize) -> Vec<u8> {
    let mut bytes = vec![0; 100 + 8 * artists];
    bytes[0] = TRACK_TEXT_CLASS;
    put(&mut bytes, 8, TEXT);
    bytes[12..12 + title.len()].copy_from_slice(title.as_bytes());
    put(&mut bytes, 96, artists as u32);
    bytes
}

fn artist_bytes(name: &str) -> Vec<u8> {
    let mut bytes = vec![0; 76];
    bytes[0] = ARTIST_CLASS;
    bytes[5..5 + name.len()].copy_from_slice(name.as_bytes());
    bytes
}

#[test]
fn a_track_names_its_text_which_spells_its_title_and_its_artists() {
    let mut body = Body::new(AudioDataKind::Music);
    body.item(ARTISTS[0], artist_bytes("THE_FIRST"), &[], &[]);
    body.item(ARTISTS[1], artist_bytes("ANOTHER"), &[], &[]);
    // The third artist is one the body does not hold.
    let rows = [(100, ARTISTS[0]), (108, 0xA271_0009), (116, ARTISTS[1])];
    body.item(TEXT, text_bytes("A_MADE_UP_SONG", 3), &rows, &[]);
    body.item(
        TRACKS[0],
        track_bytes(),
        &[(12, SOUNDS[0]), (16, TEXT)],
        &[],
    );
    body.item(LISTS[0], list_bytes(1), &[(76, TRACKS[0])], &[]);
    body.item(STATION, station_bytes(1, 0, 0), &[(64, LISTS[0])], &[]);
    let data = body.done();
    let read = stations(&data).expect("read");
    let track = read[0].lists[0].tracks.as_ref().expect("the list")[0];
    assert_eq!(track.text, Some(TEXT));
    let want = TrackText {
        title: "A_MADE_UP_SONG".into(),
        artists: vec!["THE_FIRST".into(), "ANOTHER".into()],
    };
    assert_eq!(track_texts(&data).expect("texts").get(&TEXT), Some(&want));

    let refused = |text: Vec<u8>, rows: &[(usize, u32)]| {
        let mut body = Body::new(AudioDataKind::Music);
        body.item(ARTISTS[0], vec![ARTIST_CLASS; 76], &[], &[]);
        body.item(TEXT, text, rows, &[]);
        track_texts(&body.done()).expect_err("refused").to_string()
    };
    let mut other_key = text_bytes("A", 0);
    put(&mut other_key, 8, TEXT + 1);
    assert!(refused(other_key, &[]).contains("its own key"));
    let mut unpadded = text_bytes("A", 0);
    unpadded[40] = b'x';
    assert!(refused(unpadded, &[]).contains("no NUL-padded name at +12"));
    let mut too_many = text_bytes("A", 0);
    put(&mut too_many, 96, 9);
    assert!(refused(too_many, &[]).contains("counts 9 artists"));
    assert!(refused(text_bytes("A", 1), &[]).contains("no reference row at +100"));
    let why = refused(text_bytes("A", 1), &[(100, ARTISTS[0])]);
    assert!(why.contains("holds no NUL-padded name at +5"), "{why}");
    let sounds = Body::new(AudioDataKind::Sounds).done();
    assert_eq!(track_texts(&sounds), Err(RadioError::NotMusic("sounds")));
}

#[test]
fn the_list_of_every_station_and_a_map_of_stations_alone_list_stations() {
    let stations_of = [STATION, STATION + 1, STATION + 2];
    let station_list = |count: u8, rows: usize| {
        let mut bytes = vec![0; 8 + 4 * rows];
        bytes[0] = STATION_LIST_CLASS;
        bytes[5] = count;
        bytes
    };
    let map = |entries: usize| {
        let mut bytes = vec![0xFF; 12 + 8 * entries];
        bytes[0] = STATION_MAP_CLASS;
        put(&mut bytes, 8, entries as u32);
        bytes
    };
    let body = |list: Vec<u8>, rows: &[(usize, u32)]| {
        let mut body = Body::new(AudioDataKind::Music);
        for key in stations_of {
            let mut station = station_bytes(0, 0, 0);
            put(&mut station, 8, key);
            body.item(key, station, &[], &[]);
        }
        body.item(TRACKS[0], track_bytes(), &[(12, SOUNDS[0])], &[]);
        // A map of stations, a map that names a track too, and a map with an entry of no row.
        body.item(
            0x4A90_0001,
            map(2),
            &[(16, STATION + 2), (24, STATION)],
            &[],
        );
        body.item(0x4A90_0002, map(2), &[(16, STATION), (24, TRACKS[0])], &[]);
        body.item(0x4A90_0003, map(2), &[(16, STATION)], &[]);
        body.item(0x1157_AAAA, list, rows, &[]);
        body.done()
    };
    let rows = [(8, STATION + 1), (12, STATION), (16, STATION + 2)];
    let read = station_lists(&body(station_list(3, 3), &rows)).expect("read");
    let want = [
        StationList {
            key: 0x4A90_0001,
            ordered: false,
            stations: vec![STATION + 2, STATION],
        },
        StationList {
            key: 0x1157_AAAA,
            ordered: true,
            stations: vec![STATION + 1, STATION, STATION + 2],
        },
    ];
    assert_eq!(read, want);

    let refused = |list: Vec<u8>, rows: &[(usize, u32)]| {
        (station_lists(&body(list, rows)).expect_err("refused")).to_string()
    };
    assert!(refused(station_list(4, 3), &rows).contains("counts 4 stations"));
    let why = refused(
        station_list(3, 3),
        &[(8, STATION), (12, TRACKS[0]), (16, STATION)],
    );
    assert!(why.contains("names no station at +12"), "{why}");
    let sounds = Body::new(AudioDataKind::Sounds).done();
    assert!(station_lists(&sounds).is_err());
}

#[test]
fn a_list_or_a_track_the_body_does_not_hold_is_kept_by_its_key() {
    let mut body = Body::new(AudioDataKind::Music);
    body.item(LISTS[0], list_bytes(1), &[(76, TRACKS[0])], &[]);
    let rows = [(64, LISTS[0]), (76, LISTS[1])];
    body.item(STATION, station_bytes(2, 0, 0), &rows, &[]);
    let read = stations(&body.done()).expect("read");
    let absent = Track {
        key: TRACKS[0],
        sound: None,
        text: None,
    };
    assert_eq!(read[0].lists[0].tracks, Some(vec![absent]));
    assert_eq!(read[0].lists[1].tracks, None);
}

#[test]
fn a_station_that_leaves_its_layout_is_refused() {
    let refused = |station: Vec<u8>, rows: &[(usize, u32)]| {
        let mut body = music();
        // The station is the last item: another takes its place.
        let start = body.index.pop().expect("the station").1 as usize;
        body.reference_fields.retain(|f| (*f as usize) < start + 8);
        body.data.truncate(start);
        body.item(STATION, station, rows, &[]);
        stations(&body.done()).expect_err("refused")
    };
    let rows = [(64, LISTS[0]), (76, LISTS[1])];
    let intact = station_bytes(2, 1, 1);

    let mut other_key = intact.clone();
    put(&mut other_key, 8, STATION + 1);
    let why = refused(other_key, &rows).to_string();
    assert!(why.contains("its own key"), "{why}");

    let mut too_many = intact.clone();
    put(&mut too_many, 60, 200);
    let why = refused(too_many, &rows).to_string();
    assert!(why.contains("counts 200 entries at +60"), "{why}");

    let mut pairs = intact.clone();
    put(&mut pairs, 64 + 24, 9);
    let why = refused(pairs, &rows).to_string();
    assert!(why.contains("counts 9 entries at +88"), "{why}");

    // A list's word that the reference fields do not list is no row.
    let why = refused(intact, &rows[..1]).to_string();
    assert!(why.contains("no reference row at +76"), "{why}");
}

#[test]
fn a_list_or_track_of_another_class_or_shape_is_refused() {
    let mut body = Body::new(AudioDataKind::Music);
    body.item(LISTS[0], track_bytes(), &[(12, SOUNDS[0])], &[]);
    body.item(STATION, station_bytes(1, 0, 0), &[(64, LISTS[0])], &[]);
    let refused = stations(&body.done()).expect_err("refused");
    assert!(
        matches!(refused, RadioError::TrackList { key, .. } if key == LISTS[0]),
        "{refused}"
    );

    let mut wide_count = list_bytes(1);
    put(&mut wide_count, 72, 0x0100);
    let mut body = Body::new(AudioDataKind::Music);
    body.item(LISTS[0], wide_count, &[], &[]);
    body.item(STATION, station_bytes(1, 0, 0), &[(64, LISTS[0])], &[]);
    let why = stations(&body.done()).expect_err("refused").to_string();
    assert!(why.contains("not one byte"), "{why}");

    let mut body = Body::new(AudioDataKind::Music);
    body.item(TRACKS[0], track_bytes(), &[], &[]);
    body.item(LISTS[0], list_bytes(1), &[(76, TRACKS[0])], &[]);
    body.item(STATION, station_bytes(1, 0, 0), &[(64, LISTS[0])], &[]);
    let refused = stations(&body.done()).expect_err("refused");
    assert!(
        matches!(refused, RadioError::Track { key, .. } if key == TRACKS[0]),
        "{refused}"
    );

    let sounds = Body::new(AudioDataKind::Sounds).done();
    assert_eq!(stations(&sounds), Err(RadioError::NotMusic("sounds")));
}

const WAVES: [u32; 2] = [0xE000_0123, 0x2000_0456];
const CHILDREN: [u32; 2] = [0xC41D_0001, 0xC41D_0002];

/// A sounds body: a streamed sound of two channels whose sounds each name a wave, and one of a
/// channel whose sound the body does not hold. Each item's header states no field.
fn sounds() -> Body {
    let mut body = Body::new(AudioDataKind::Sounds);
    for (child, wave) in CHILDREN.iter().zip(WAVES) {
        let mut bytes = vec![12, 0];
        bytes.extend(0x0BA2_0000u32.to_le_bytes());
        bytes.extend(wave.to_le_bytes());
        bytes.extend([0, 0, 0]);
        body.item(*child, bytes, &[], &[2]);
    }
    let streamed = |duration: u32, channels: usize| {
        let mut bytes = vec![STREAMED_SOUND_CLASS, 0];
        bytes.extend(duration.to_le_bytes());
        bytes.extend([12, channels as u8]);
        bytes.resize(8 + 4 * channels, 0);
        bytes
    };
    let rows = [(8, CHILDREN[0]), (12, CHILDREN[1])];
    body.item(SOUNDS[0], streamed(183_250, 2), &rows, &[]);
    body.item(SOUNDS[1], streamed(9_000, 1), &[(8, 0xAB5E_0001)], &[]);
    body
}

#[test]
fn a_streamed_sound_states_its_length_and_its_channels_streams() {
    let (read, unread) = streamed_sounds(&sounds().done()).expect("read");
    assert_eq!(unread, 0);
    let first = StreamedSound {
        key: SOUNDS[0],
        duration_ms: 183_250,
        channels: vec![
            (CHILDREN[0], Some(0x0000_0123)),
            (CHILDREN[1], Some(0x0000_0456)),
        ],
    };
    assert_eq!(read.get(&SOUNDS[0]), Some(&first));
    let second = &read[&SOUNDS[1]];
    assert_eq!(second.channels, [(0xAB5E_0001, None)]);
    assert_eq!(second.streams().count(), 0);

    let music = Body::new(AudioDataKind::Music).done();
    assert_eq!(
        streamed_sounds(&music).map(|r| r.1),
        Err(RadioError::NotSounds("music"))
    );
}

#[test]
fn a_streamed_sound_no_layout_reads_is_counted_and_not_listed() {
    let mut body = sounds();
    // A channel row the reference fields do not list.
    let mut bytes = vec![STREAMED_SOUND_CLASS, 0, 1, 0, 0, 0, 12, 1];
    bytes.extend(0u32.to_le_bytes());
    body.item(SOUNDS[2], bytes, &[], &[]);
    let (read, unread) = streamed_sounds(&body.done()).expect("read");
    assert_eq!(unread, 1);
    assert!(!read.contains_key(&SOUNDS[2]));
}

#[test]
fn a_station_s_tracks_join_the_streamed_sounds_they_play() {
    let station = &stations(&music().done()).expect("stations")[0];
    let (sounds, _) = streamed_sounds(&sounds().done()).expect("sounds");
    let joined: Vec<(u32, Option<u32>, Vec<u32>)> = (station.tracks(&sounds))
        .map(|(track, sound)| {
            let streams = sound.map(|s| s.streams().collect()).unwrap_or_default();
            (track.key, sound.map(|s| s.duration_ms), streams)
        })
        .collect();
    let want = [
        (TRACKS[0], Some(183_250), vec![0x123, 0x456]),
        (TRACKS[1], Some(9_000), vec![]),
        (TRACKS[2], None, vec![]),
    ];
    assert_eq!(joined, want);
}

#[test]
fn a_flags_word_states_two_bits_a_flag_and_the_third_hides_a_station() {
    // 0b01 is set, 0b00 clear, and 0b10 not stated.
    let word = 0b10_01_00_01;
    let read = [0, 1, 2, 3].map(|n| flag(word, n));
    assert_eq!(read, [Some(true), Some(false), Some(true), None]);
    assert_eq!(flag(word, 16), None, "past the word's sixteen flags");
    // The fixture fills a station with 0x11, which sets every other flag.
    let station = &stations(&music().done()).expect("stations")[0];
    assert_eq!(station.flags, 0x1111_1111);
    assert!(station.hidden());
}

#[test]
fn a_track_id_keys_its_title_and_its_artist() {
    let (title, artist) = track_text_keys(1234);
    assert_eq!(title, name_hash32(b"1234S"));
    assert_eq!(artist, name_hash32(b"1234A"));
}

#[test]
fn a_station_s_name_is_found_among_known_names_whole_or_by_its_words() {
    let stations = [
        "radio_made_up_fm",
        "hidden_radio_made_up_fm",
        "hidden_radio_corner_shop",
        "radio_never_spelled",
    ];
    let keys: BTreeSet<u32> = (stations.iter())
        .map(|name| name_hash32(name.as_bytes()))
        .collect();
    // A name whole, and two words in a row of a longer one.
    let known = ["made_up_fm", "prop_corner_shop_sign_01", "unrelated"];
    let found = station_names(&keys, known.into_iter());
    for name in &stations[..3] {
        let named = found.get(&name_hash32(name.as_bytes()));
        assert_eq!(named.map(String::as_str), Some(*name));
    }
    assert_eq!(
        found.len(),
        3,
        "a station no known name spells stays unnamed"
    );
    assert_eq!(name_of_text("The Made-Up  FM 98.3!"), "the_made_up_fm_98_3");
}
