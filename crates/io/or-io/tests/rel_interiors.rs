//! The interiors of a game data `.rel` and their rooms, added, edited and removed, on a file laid
//! out by hand as the installs lay theirs out
//! (`the_interiors_of_an_install_lay_out_as_an_edit_writes_them`): an item added reads back typed
//! and equal, a save reaches its fixed point, every other item keeps its bytes and its listed words,
//! and each refusal leaves the file as it was.

use std::collections::BTreeMap;

use or_io::hash::string_hash_str;
use or_io::rel::game_object::{
    AlarmSettings, GameObjectItem, InteriorRoom, InteriorSettings, PedVoiceGroups, Position, Room,
    StaticEmitter,
};
use or_io::rel::layout::{NameTableOffset, Vector3};
use or_io::rel::{RelFile, RelIndex, RelIndexHash, RelIndexName};

/// What the hand-built data block opens with, which an edit carries.
const LEAD: [u8; 4] = [0xD0, 0x0D, 0x5E, 0x00];

fn key(name: &str) -> u32 {
    string_hash_str(name)
}

/// A room whose every member differs from its neighbours', so a member read from another's bytes shows.
fn room(name: &str) -> InteriorRoom {
    InteriorRoom {
        name_table_offset: NameTableOffset(key(name) & 0xFF_FFFF),
        flags: 0xAAAA_AAAA,
        room_name: key(&format!("{name}_in_the_mlo")),
        ambient_zone: key(&format!("{name}_zone")),
        interior_type: 2,
        reverb_small: 0.25,
        reverb_medium: 0.5,
        reverb_large: 0.75,
        room_tone_sound: key("room_tone"),
        rain_type: 1,
        exterior_audibility: 1.5,
        room_occlusion_damping: 0.125,
        non_marked_portal_occlusion: 0.375,
        distance_from_portal_for_occlusion: 3.0,
        distance_from_portal_fade_distance: 4.0,
        weapon_metrics: key("weapon_metrics"),
        interior_walla_sound_set: key("room_walla"),
    }
}

fn interior(name: &str, rooms: &[&str]) -> InteriorSettings {
    InteriorSettings {
        name_table_offset: NameTableOffset(key(name) & 0xFF_FFFF),
        flags: 0x5555_5555,
        interior_walla_sound_set: key("interior_walla"),
        interior_reflections: key("reflections"),
        room: rooms.iter().map(|room| Room { r#ref: key(room) }).collect(),
    }
}

fn boxed_room(name: &str) -> GameObjectItem {
    GameObjectItem::InteriorRoom(Box::new(room(name)))
}

fn boxed_interior(name: &str, rooms: &[&str]) -> GameObjectItem {
    GameObjectItem::InteriorSettings(Box::new(interior(name, rooms)))
}

/// Where `body` states `value`, which it must state once.
fn word_at(body: &[u8], value: u32) -> u32 {
    let at: Vec<usize> = body
        .windows(4)
        .enumerate()
        .filter(|(_, word)| *word == value.to_le_bytes())
        .map(|(at, _)| at)
        .collect();
    assert_eq!(at.len(), 1, "{value:#x} is stated {} times", at.len());
    at[0] as u32
}

/// Where `body` states `value`, the `nth` time it does.
fn nth_word_at(body: &[u8], value: u32, nth: usize) -> u32 {
    body.windows(4)
        .enumerate()
        .filter(|(_, word)| *word == value.to_le_bytes())
        .map(|(at, _)| at as u32)
        .nth(nth)
        .unwrap_or_else(|| panic!("{value:#x} is not stated {} times", nth + 1))
}

/// The words of an item the hash table lists, found by their values and not by a stated offset.
fn listed_words(item: &GameObjectItem, body: &[u8]) -> Vec<u32> {
    match item {
        // A room an interior lists twice is stated twice, in the order listed.
        GameObjectItem::InteriorSettings(interior) => interior
            .room
            .iter()
            .enumerate()
            .map(|(i, room)| {
                let earlier = interior.room[..i].iter().filter(|r| *r == room).count();
                nth_word_at(body, room.r#ref, earlier)
            })
            .collect(),
        GameObjectItem::InteriorRoom(room) => vec![word_at(body, room.ambient_zone)],
        GameObjectItem::AlarmSettings(alarm) => vec![word_at(body, alarm.alarm_loop)],
        _ => Vec::new(),
    }
}

fn voices(name_table_offset: u32, ringtone: &str) -> GameObjectItem {
    GameObjectItem::PedVoiceGroups(Box::new(PedVoiceGroups {
        name_table_offset: NameTableOffset(name_table_offset),
        pvg_type: 1,
        pvg_bits: 2,
        voice_priority: 3,
        ringtone_sounds: key(ringtone),
        primary_voice: Vec::new(),
        mini_voice: Vec::new(),
        gang_voice: Vec::new(),
        backup_pvg: Vec::new(),
    }))
}

/// Two interiors that share a room, an emitter and an alarm that name the first, and two packed
/// items of 15 bytes first, so the second starts on an odd byte and the items after it need padding.
fn hand_built_items() -> Vec<(u32, GameObjectItem)> {
    let alarm = GameObjectItem::AlarmSettings(Box::new(AlarmSettings {
        name_table_offset: NameTableOffset(9),
        alarm_loop: key("alarm_loop"),
        alarm_decay_curve: key("decay"),
        stop_distance: 100,
        interior_settings: key("lobby"),
        bank_name: key("bank"),
        centre_position: Vector3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            w: 0.0,
        },
    }));
    let radio = GameObjectItem::StaticEmitter(Box::new(StaticEmitter {
        name_table_offset: NameTableOffset(11),
        flags: 0xAAAA_AAAA,
        sound: key("emitter_sound"),
        radio_station: key("station"),
        position: Position {
            x: 4.0,
            y: 5.0,
            z: 6.0,
        },
        min_distance: 1.0,
        max_distance: 20.0,
        emitted_volume: -300,
        filter_cutoff: 20_000,
        hpf_cutoff: 100,
        rolloff_factor: 200,
        interior_settings: key("lobby"),
        interior_room: key("lobby_hall"),
        radio_station_for_score: 0,
        max_leakage: 0.5,
        min_leakage_distance: 1,
        max_leakage_distance: 2,
        alarm_settings: key("alarm"),
        on_break_one_shot: 0,
        max_path_depth: 3,
        small_reverb_send: 4,
        medium_reverb_send: 5,
        large_reverb_send: 6,
        min_time_minutes: 7,
        max_time_minutes: 8,
        broken_health: 0.25,
        undamaged_health: 0.75,
    }));
    vec![
        (key("voices"), voices(7, "ringtone")),
        (key("more_voices"), voices(8, "another_ringtone")),
        (
            key("lobby"),
            boxed_interior("lobby", &["lobby_hall", "shared_stairs"]),
        ),
        (key("lobby_hall"), boxed_room("lobby_hall")),
        (key("shared_stairs"), boxed_room("shared_stairs")),
        (
            key("cellar"),
            boxed_interior("cellar", &["cellar_room", "shared_stairs"]),
        ),
        (key("cellar_room"), boxed_room("cellar_room")),
        (key("radio"), radio),
        (key("alarm"), alarm),
    ]
}

fn hand_built() -> RelFile {
    hand_built_of(hand_built_items())
}

/// A game data file of `items` in this order, written by `RelFile::save` and opened again.
fn hand_built_of(items: Vec<(u32, GameObjectItem)>) -> RelFile {
    let mut data = LEAD.to_vec();
    let (mut index, mut hash_offsets, mut pack_offsets) = (Vec::new(), Vec::new(), Vec::new());
    for (key, item) in items {
        let body = item.encode().unwrap();
        data.resize(data.len().next_multiple_of(item.own_align()), 0);
        let offset = data.len() as u32;
        hash_offsets.extend(listed_words(&item, &body).iter().map(|at| offset + at));
        if let GameObjectItem::StaticEmitter(emitter) = &item {
            pack_offsets.push(offset + word_at(&body, emitter.sound));
        }
        index.push(RelIndexHash {
            hash: key,
            offset,
            length: body.len() as u32,
        });
        data.extend_from_slice(&body);
    }
    index.sort_by_key(|entry| (entry.hash & 0xFF, entry.hash));
    let file = RelFile {
        rel_type: 151,
        data,
        names: Vec::new(),
        index: RelIndex::Hashed(index),
        hash_offsets,
        pack_offsets,
    };
    RelFile::open(&file.save().unwrap()).unwrap()
}

fn hashed(file: &RelFile) -> &[RelIndexHash] {
    match &file.index {
        RelIndex::Hashed(index) => index,
        RelIndex::Named(_) => panic!("a game data file keyed by name"),
    }
}

/// The item table's entry of the item named `name`.
fn entry(file: &RelFile, name: &str) -> RelIndexHash {
    *hashed(file)
        .iter()
        .find(|entry| entry.hash == key(name))
        .unwrap_or_else(|| panic!("no item {name}"))
}

/// An item's bytes and the words of it the hash table and the pack table list, counted from its start.
type Held = (Vec<u8>, Vec<u32>, Vec<u32>);

/// Every item of the file by its key.
fn held(file: &RelFile) -> BTreeMap<u32, Held> {
    hashed(file)
        .iter()
        .map(|entry| {
            let span = entry.offset..entry.offset + entry.length;
            let within = |offsets: &[u32]| -> Vec<u32> {
                offsets
                    .iter()
                    .filter(|&&at| span.contains(&at))
                    .map(|&at| at - entry.offset)
                    .collect()
            };
            let body = file.data[span.start as usize..span.end as usize].to_vec();
            (
                entry.hash,
                (body, within(&file.hash_offsets), within(&file.pack_offsets)),
            )
        })
        .collect()
}

/// Holds the file to the layout the installs have, to its fixed point, and each interior and room to its listed words.
fn assert_sound(file: &RelFile) {
    assert_eq!(file.data[..4], LEAD);
    let mut spans: Vec<&RelIndexHash> = hashed(file).iter().collect();
    spans.sort_by_key(|entry| entry.offset);
    let mut end = LEAD.len();
    for span in &spans {
        let start = span.offset as usize;
        let body = &file.data[start..start + span.length as usize];
        let item = GameObjectItem::decode(body).unwrap();
        assert_eq!(
            start,
            end.next_multiple_of(item.own_align()),
            "{:#x}",
            span.hash
        );
        assert!(
            file.data[end..start].iter().all(|&b| b == 0),
            "{:#x}",
            span.hash
        );
        end = start + body.len();
        if matches!(
            item,
            GameObjectItem::InteriorSettings(_) | GameObjectItem::InteriorRoom(_)
        ) {
            let (_, hash, pack) = &held(file)[&span.hash];
            assert_eq!(*hash, listed_words(&item, body), "{:#x}", span.hash);
            assert!(pack.is_empty());
        }
    }
    assert_eq!(end, file.data.len());
    let order: Vec<(u32, u32)> = hashed(file)
        .iter()
        .map(|e| (e.hash & 0xFF, e.hash))
        .collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{order:x?}");
    assert!(file.hash_offsets.windows(2).all(|pair| pair[0] < pair[1]));
    let listed: usize = held(file)
        .values()
        .map(|(_, hash, pack)| hash.len() + pack.len())
        .sum();
    assert_eq!(listed, file.hash_offsets.len() + file.pack_offsets.len());

    let saved = file.save().unwrap();
    let again = RelFile::open(&saved).unwrap();
    assert_eq!(&again, file);
    assert_eq!(again.save().unwrap(), saved);
}

/// `after` less the items under `changed`, which must be every other item of `before`, untouched.
fn assert_others_untouched(before: &RelFile, after: &RelFile, changed: &[u32]) {
    let mut was = held(before);
    let mut is = held(after);
    for key in changed {
        was.remove(key);
        is.remove(key);
    }
    assert_eq!(was, is);
}

#[track_caller]
fn refusal(result: or_io::Result<()>) -> String {
    result.unwrap_err().to_string()
}

#[test]
fn the_hand_built_file_is_one_an_edit_takes() {
    let file = hand_built();
    assert_sound(&file);
    // A packed item starts where the item before it ends, on any byte, and an interior at the
    // next multiple of four: stated here by the bytes, where `assert_sound` asks `own_align`.
    let (first, second, lobby) = (
        entry(&file, "voices"),
        entry(&file, "more_voices"),
        entry(&file, "lobby"),
    );
    assert_eq!(first.offset as usize, LEAD.len());
    assert_eq!(second.offset, first.offset + first.length);
    assert_ne!(second.offset % 4, 0);
    let end = second.offset + second.length;
    assert_eq!(lobby.offset, end.next_multiple_of(4));
    assert_ne!(lobby.offset, end);
    assert_eq!(
        file.interior(key("lobby")).unwrap(),
        Some(interior("lobby", &["lobby_hall", "shared_stairs"]))
    );
    assert_eq!(
        file.interior_room(key("cellar_room")).unwrap(),
        Some(room("cellar_room"))
    );
    assert_eq!(file.interior(key("no_such_item")).unwrap(), None);
    assert_eq!(file.interior_room(key("no_such_item")).unwrap(), None);
    let why = file.interior(key("lobby_hall")).unwrap_err().to_string();
    assert!(
        why.contains("is an InteriorRoom, not an InteriorSettings"),
        "{why}"
    );
    let why = file.interior_room(key("alarm")).unwrap_err().to_string();
    assert!(
        why.contains("is an AlarmSettings, not an InteriorRoom"),
        "{why}"
    );
}

#[test]
fn an_interior_and_its_room_added_from_nothing_read_back_typed_and_equal() {
    let before = hand_built();
    let mut file = before.clone();
    file.add_interior(key("vault"), &interior("vault", &[]))
        .unwrap();
    assert_sound(&file);
    assert_eq!(
        file.interior(key("vault")).unwrap(),
        Some(interior("vault", &[]))
    );
    assert_others_untouched(&before, &file, &[key("vault")]);

    file.add_interior_room(key("vault"), key("vault_door"), &room("vault_door"))
        .unwrap();
    file.add_interior_room(key("vault"), key("vault_floor"), &room("vault_floor"))
        .unwrap();
    assert_sound(&file);
    let file = RelFile::open(&file.save().unwrap()).unwrap();
    assert_eq!(
        file.interior(key("vault")).unwrap(),
        Some(interior("vault", &["vault_door", "vault_floor"]))
    );
    assert_eq!(
        file.interior_room(key("vault_door")).unwrap(),
        Some(room("vault_door"))
    );
    assert_eq!(
        file.interior_room(key("vault_floor")).unwrap(),
        Some(room("vault_floor"))
    );
    assert_others_untouched(
        &before,
        &file,
        &[key("vault"), key("vault_door"), key("vault_floor")],
    );
    assert_eq!(hashed(&file).len(), hashed(&before).len() + 3);
}

#[test]
fn a_room_joins_an_interior_the_file_already_held_and_only_that_interior_changes() {
    let before = hand_built();
    let mut file = before.clone();
    file.add_interior_room(key("cellar"), key("cellar_store"), &room("cellar_store"))
        .unwrap();
    assert_sound(&file);
    assert_eq!(
        file.interior(key("cellar")).unwrap(),
        Some(interior(
            "cellar",
            &["cellar_room", "shared_stairs", "cellar_store"]
        ))
    );
    assert_others_untouched(&before, &file, &[key("cellar"), key("cellar_store")]);
    // The interior grew by one word, and the room and the emitter after it moved by it, with the
    // word the hash table lists of the one and the pack table of the other.
    let (was, is) = (held(&before), held(&file));
    assert_eq!(is[&key("cellar")].0.len(), was[&key("cellar")].0.len() + 4);
    for moved in ["cellar_room", "radio"] {
        assert_eq!(
            entry(&file, moved).offset,
            entry(&before, moved).offset + 4,
            "{moved}"
        );
    }
    assert_eq!(is[&key("cellar_room")].1.len(), 1);
    assert_eq!(is[&key("alarm")].1.len(), 1);
    assert_eq!(is[&key("radio")].2.len(), 1);
}

#[test]
fn a_copy_takes_an_interior_and_its_rooms_under_new_keys_and_removing_it_gives_the_first_bytes_back(
) {
    let before = hand_built();
    let mut file = before.clone();
    file.copy_interior(
        key("lobby"),
        key("lobby_two"),
        &[
            (key("lobby_hall"), key("lobby_two_hall")),
            (key("shared_stairs"), key("lobby_two_stairs")),
        ],
    )
    .unwrap();
    assert_sound(&file);
    let file = RelFile::open(&file.save().unwrap()).unwrap();
    let mut copy = interior("lobby", &["lobby_two_hall", "lobby_two_stairs"]);
    assert_eq!(
        file.interior(key("lobby_two")).unwrap().as_ref(),
        Some(&copy)
    );
    assert_eq!(
        file.interior_room(key("lobby_two_hall")).unwrap(),
        Some(room("lobby_hall"))
    );
    assert_eq!(
        file.interior_room(key("lobby_two_stairs")).unwrap(),
        Some(room("shared_stairs"))
    );
    let added = [
        key("lobby_two"),
        key("lobby_two_hall"),
        key("lobby_two_stairs"),
    ];
    assert_others_untouched(&before, &file, &added);

    // A typed member of the copy is edited, and nothing else moves.
    copy.interior_walla_sound_set = key("another_walla");
    copy.flags = 1;
    let mut edited = file.clone();
    edited.set_interior(key("lobby_two"), &copy).unwrap();
    assert_sound(&edited);
    assert_eq!(edited.interior(key("lobby_two")).unwrap(), Some(copy));
    assert_others_untouched(&file, &edited, &[key("lobby_two")]);
    assert_eq!(hashed(&edited), hashed(&file));

    edited.remove_interior(key("lobby_two")).unwrap();
    assert_eq!(edited, before);
    assert_eq!(edited.save().unwrap(), before.save().unwrap());
}

#[test]
fn a_typed_member_of_a_room_is_edited_and_every_other_byte_stays() {
    let before = hand_built();
    let mut file = before.clone();
    let mut edited = room("lobby_hall");
    edited.reverb_large = 0.9;
    edited.ambient_zone = key("another_zone");
    edited.interior_type = 3;
    file.set_interior_room(key("lobby_hall"), &edited).unwrap();
    assert_sound(&file);
    assert_eq!(file.interior_room(key("lobby_hall")).unwrap(), Some(edited));
    assert_others_untouched(&before, &file, &[key("lobby_hall")]);
    assert_eq!(hashed(&file), hashed(&before));
    assert_eq!(file.hash_offsets, before.hash_offsets);

    // An edit that changes nothing changes no byte.
    let mut same = before.clone();
    same.set_interior_room(key("lobby_hall"), &room("lobby_hall"))
        .unwrap();
    same.set_interior(
        key("lobby"),
        &interior("lobby", &["lobby_hall", "shared_stairs"]),
    )
    .unwrap();
    assert_eq!(same, before);
}

#[test]
fn removing_a_room_takes_it_out_of_every_interior_that_lists_it() {
    let before = hand_built();
    let mut file = before.clone();
    file.remove_interior_room(key("shared_stairs")).unwrap();
    assert_sound(&file);
    assert_eq!(file.interior_room(key("shared_stairs")).unwrap(), None);
    assert_eq!(
        file.interior(key("lobby")).unwrap(),
        Some(interior("lobby", &["lobby_hall"]))
    );
    assert_eq!(
        file.interior(key("cellar")).unwrap(),
        Some(interior("cellar", &["cellar_room"]))
    );
    assert_others_untouched(
        &before,
        &file,
        &[key("shared_stairs"), key("lobby"), key("cellar")],
    );
    assert_eq!(hashed(&file).len(), hashed(&before).len() - 1);
}

#[test]
fn removing_an_interior_takes_the_rooms_only_it_lists_and_keeps_a_shared_one() {
    let before = hand_built();
    let mut file = before.clone();
    file.remove_interior(key("cellar")).unwrap();
    assert_sound(&file);
    assert_eq!(file.interior(key("cellar")).unwrap(), None);
    assert_eq!(file.interior_room(key("cellar_room")).unwrap(), None);
    assert_eq!(
        file.interior_room(key("shared_stairs")).unwrap(),
        Some(room("shared_stairs"))
    );
    assert_others_untouched(&before, &file, &[key("cellar"), key("cellar_room")]);
    assert_eq!(hashed(&file).len(), hashed(&before).len() - 2);
}

#[test]
fn a_key_the_file_holds_is_refused() {
    let before = hand_built();
    let mut file = before.clone();
    let why = refusal(file.add_interior(key("cellar"), &interior("cellar", &[])));
    assert!(
        why.contains("already holds an item") && why.contains("an InteriorSettings"),
        "{why}"
    );
    let why = refusal(file.add_interior(key("alarm"), &interior("alarm", &[])));
    assert!(
        why.contains("already holds an item") && why.contains("an AlarmSettings"),
        "{why}"
    );
    let why =
        refusal(file.add_interior_room(key("cellar"), key("lobby_hall"), &room("lobby_hall")));
    assert!(
        why.contains("already holds an item") && why.contains("an InteriorRoom"),
        "{why}"
    );
    let pairs = [
        (key("lobby_hall"), key("new_hall")),
        (key("shared_stairs"), key("new_stairs")),
    ];
    let why = refusal(file.copy_interior(key("lobby"), key("cellar"), &pairs));
    assert!(why.contains("already holds an item"), "{why}");
    let taken = [pairs[0], (key("shared_stairs"), key("radio"))];
    let why = refusal(file.copy_interior(key("lobby"), key("lobby_two"), &taken));
    assert!(
        why.contains("already holds an item") && why.contains("a StaticEmitter"),
        "{why}"
    );
    assert_eq!(file, before);
}

#[test]
fn a_room_naming_an_interior_the_file_does_not_hold_is_refused() {
    let before = hand_built();
    let mut file = before.clone();
    let why = refusal(file.add_interior_room(key("attic"), key("attic_room"), &room("attic_room")));
    assert!(
        why.contains("names the interior") && why.contains("which the file does not hold"),
        "{why}"
    );
    let why = refusal(file.add_interior_room(key("alarm"), key("attic_room"), &room("attic_room")));
    assert!(
        why.contains("is an AlarmSettings, not an InteriorSettings"),
        "{why}"
    );
    assert_eq!(file, before);
}

#[test]
fn an_interior_listing_a_room_the_file_does_not_hold_is_refused() {
    let before = hand_built();
    let mut file = before.clone();
    let why = refusal(file.add_interior(key("attic"), &interior("attic", &["attic_room"])));
    assert!(
        why.contains("lists the room") && why.contains("which the file does not hold"),
        "{why}"
    );
    let why = refusal(file.set_interior(key("cellar"), &interior("cellar", &["alarm"])));
    assert!(
        why.contains("which is an AlarmSettings and not an InteriorRoom"),
        "{why}"
    );
    assert_eq!(file, before);

    // A room the file holds is listed by a second interior without a copy.
    file.add_interior(key("attic"), &interior("attic", &["cellar_room"]))
        .unwrap();
    assert_sound(&file);
    assert_eq!(
        file.interior(key("attic")).unwrap(),
        Some(interior("attic", &["cellar_room"]))
    );
}

#[test]
fn a_member_past_what_the_encoding_states_is_refused() {
    let before = hand_built();
    let mut file = before.clone();
    // A count of one byte states 255 rooms and no more.
    let mut full = interior("cellar", &[]);
    full.room = vec![
        Room {
            r#ref: key("cellar_room")
        };
        255
    ];
    file.set_interior(key("cellar"), &full).unwrap();
    assert_eq!(
        file.interior(key("cellar")).unwrap().unwrap().room.len(),
        255
    );
    assert_eq!(RelFile::open(&file.save().unwrap()).unwrap(), file);
    let filled = file.clone();
    let why = refusal(file.add_interior_room(key("cellar"), key("one_more"), &room("one_more")));
    assert!(
        why.contains("256 rooms") && why.contains("at most 255"),
        "{why}"
    );
    full.room.push(Room {
        r#ref: key("cellar_room"),
    });
    let why = refusal(file.set_interior(key("cellar"), &full));
    assert!(
        why.contains("256 rooms") && why.contains("at most 255"),
        "{why}"
    );
    assert_eq!(file, filled);

    // A name table offset shares its word with the class id and has 24 bits.
    let mut file = before.clone();
    let mut wide = room("lobby_hall");
    wide.name_table_offset = NameTableOffset(1 << 24);
    let why = refusal(file.set_interior_room(key("lobby_hall"), &wide));
    assert!(why.contains("past 24 bits"), "{why}");
    let why = refusal(file.add_interior_room(key("lobby"), key("wide_room"), &wide));
    assert!(why.contains("past 24 bits"), "{why}");
    let mut wide = interior("wide", &[]);
    wide.name_table_offset = NameTableOffset(1 << 24);
    let why = refusal(file.add_interior(key("wide"), &wide));
    assert!(why.contains("past 24 bits"), "{why}");
    assert_eq!(file, before);
}

#[test]
fn an_interior_or_a_room_another_item_names_is_not_removed() {
    let before = hand_built();
    let mut file = before.clone();
    let why = refusal(file.remove_interior(key("lobby")));
    assert!(
        why.contains("names the interior") && why.contains("cannot go"),
        "{why}"
    );
    let why = refusal(file.remove_interior_room(key("lobby_hall")));
    assert!(
        why.contains("the StaticEmitter") && why.contains("names the room"),
        "{why}"
    );
    assert_eq!(file, before);

    // With the alarm pointed elsewhere the emitter holds the interior; pointed elsewhere too, it
    // still holds the room only that interior lists; and with neither, the interior goes.
    let loosened = |alarm_names: bool, emitter_names_interior: bool, emitter_names_room: bool| {
        let mut items = hand_built_items();
        for (_, item) in &mut items {
            match item {
                GameObjectItem::AlarmSettings(alarm) if !alarm_names => alarm.interior_settings = 0,
                GameObjectItem::StaticEmitter(emitter) => {
                    if !emitter_names_interior {
                        emitter.interior_settings = 0;
                    }
                    if !emitter_names_room {
                        emitter.interior_room = 0;
                    }
                }
                _ => {}
            }
        }
        hand_built_of(items)
    };
    let mut by_emitter = loosened(false, true, true);
    let why = refusal(by_emitter.remove_interior(key("lobby")));
    assert!(
        why.contains("the StaticEmitter") && why.contains("names the interior"),
        "{why}"
    );
    let mut by_alarm = loosened(true, false, false);
    let why = refusal(by_alarm.remove_interior(key("lobby")));
    assert!(
        why.contains("the AlarmSettings") && why.contains("names the interior"),
        "{why}"
    );
    let mut by_room = loosened(false, false, true);
    let held_before = by_room.clone();
    let why = refusal(by_room.remove_interior(key("lobby")));
    assert!(
        why.contains("the StaticEmitter") && why.contains("names the room"),
        "{why}"
    );
    assert_eq!(by_room, held_before);
    let mut free = loosened(false, false, false);
    free.remove_interior(key("lobby")).unwrap();
    assert_sound(&free);
    assert_eq!(free.interior_room(key("lobby_hall")).unwrap(), None);
    assert_eq!(
        free.interior_room(key("shared_stairs")).unwrap(),
        Some(room("shared_stairs"))
    );

    let why = refusal(file.remove_interior(key("no_such_item")));
    assert!(why.contains("holds no interior"), "{why}");
    let why = refusal(file.remove_interior_room(key("no_such_item")));
    assert!(why.contains("holds no room"), "{why}");
    let why = refusal(file.remove_interior(key("lobby_hall")));
    assert!(why.contains("not an InteriorSettings"), "{why}");
    let why = refusal(file.remove_interior_room(key("lobby")));
    assert!(why.contains("not an InteriorRoom"), "{why}");
    assert_eq!(file, before);
}

#[test]
fn a_copy_needs_a_key_for_each_room_the_interior_lists_and_for_no_other() {
    let before = hand_built();
    let mut file = before.clone();
    let why = refusal(file.copy_interior(
        key("lobby"),
        key("lobby_two"),
        &[(key("lobby_hall"), key("lobby_two_hall"))],
    ));
    assert!(why.contains("was given no key"), "{why}");
    let why = refusal(file.copy_interior(
        key("lobby"),
        key("lobby_two"),
        &[
            (key("lobby_hall"), key("lobby_two_hall")),
            (key("shared_stairs"), key("lobby_two_stairs")),
            (key("cellar_room"), key("lobby_two_cellar")),
        ],
    ));
    assert!(why.contains("does not list the room"), "{why}");
    let why = refusal(file.copy_interior(key("no_such_item"), key("lobby_two"), &[]));
    assert!(why.contains("holds no interior"), "{why}");
    assert_eq!(file, before);
}

#[test]
fn a_file_an_edit_would_not_write_back_as_it_is_is_refused_untouched() {
    let edit =
        |file: &mut RelFile| refusal(file.add_interior(key("vault"), &interior("vault", &[])));
    let sound = hand_built();

    let mut sounds = sound.clone();
    sounds.rel_type = 54;
    let before = sounds.clone();
    let why = edit(&mut sounds);
    assert!(
        why.contains("rel_type 151") && why.contains("rel_type 54"),
        "{why}"
    );
    assert!(sounds.interior(key("lobby")).is_err());
    assert_eq!(sounds, before);

    let mut named = sound.clone();
    named.names.push("a_name".into());
    let before = named.clone();
    let why = edit(&mut named);
    assert!(why.contains("name table"), "{why}");
    assert_eq!(named, before);

    // The item table in another order than by low byte, then key.
    let mut reordered = sound.clone();
    let RelIndex::Hashed(index) = &mut reordered.index else {
        panic!()
    };
    index.swap(0, 1);
    let before = reordered.clone();
    let why = edit(&mut reordered);
    assert!(why.contains("item table"), "{why}");
    assert_eq!(reordered, before);

    // A byte of padding between two items that is not zero.
    let mut padded = sound.clone();
    let more_voices = entry(&padded, "more_voices");
    let lobby = entry(&padded, "lobby").offset;
    assert!(
        lobby >= more_voices.offset + more_voices.length + 2,
        "the packed items leave no padding"
    );
    padded.data[lobby as usize - 1] = 1;
    let before = padded.clone();
    let why = edit(&mut padded);
    assert!(why.contains("data block"), "{why}");
    assert_eq!(padded, before);

    // A hash table that lists two items' words in another order than the items', and a pack
    // table that lists a word of the padding.
    let mut shuffled = sound.clone();
    let last = shuffled.hash_offsets.len() - 1;
    shuffled.hash_offsets.swap(0, last);
    let before = shuffled.clone();
    let why = edit(&mut shuffled);
    assert!(why.contains("hash table"), "{why}");
    assert_eq!(shuffled, before);

    let mut stray = sound.clone();
    stray.pack_offsets.push(lobby - 2);
    let why = edit(&mut stray);
    assert!(why.contains("a word no item holds"), "{why}");

    // A room whose listed word is another than its ambient zone is carried, and not written again.
    let mut odd = sound.clone();
    let hall = *hashed(&odd)
        .iter()
        .find(|entry| entry.hash == key("lobby_hall"))
        .unwrap();
    let span = hall.offset..hall.offset + hall.length;
    let zone = odd
        .hash_offsets
        .iter()
        .position(|at| span.contains(at))
        .unwrap();
    odd.hash_offsets[zone] += 4;
    let before = odd.clone();
    odd.add_interior(key("vault"), &interior("vault", &[]))
        .unwrap();
    assert_others_untouched(&before, &odd, &[key("vault")]);
    let kept = odd.clone();
    let why = refusal(odd.set_interior_room(key("lobby_hall"), &room("lobby_hall")));
    assert!(why.contains("other words"), "{why}");
    assert_eq!(odd, kept);
}

#[test]
fn a_copy_refuses_one_key_for_two_rooms() {
    let before = hand_built();
    let mut file = before.clone();
    let why = refusal(file.copy_interior(
        key("lobby"),
        key("lobby_two"),
        &[
            (key("lobby_hall"), key("one_key")),
            (key("shared_stairs"), key("one_key")),
        ],
    ));
    assert!(
        why.contains("for two of its items") && !why.contains("already holds"),
        "{why}"
    );
    // The key the copied interior goes under is not a room's too.
    let why = refusal(file.copy_interior(
        key("lobby"),
        key("lobby_two"),
        &[
            (key("lobby_hall"), key("lobby_two_hall")),
            (key("shared_stairs"), key("lobby_two")),
        ],
    ));
    assert!(
        why.contains("for two of its items") && !why.contains("already holds"),
        "{why}"
    );
    assert_eq!(file, before);
}

#[test]
fn a_copy_refuses_two_keys_for_one_room() {
    let before = hand_built();
    let mut file = before.clone();
    let why = refusal(file.copy_interior(
        key("lobby"),
        key("lobby_two"),
        &[
            (key("lobby_hall"), key("lobby_two_hall")),
            (key("shared_stairs"), key("lobby_two_stairs")),
            (key("lobby_hall"), key("another_hall")),
        ],
    ));
    assert!(why.contains("two keys"), "{why}");
    assert_eq!(file, before);
}

#[test]
fn a_copy_of_an_interior_that_lists_a_room_twice_copies_the_room_once() {
    let mut file = hand_built();
    let twice = ["cellar_room", "shared_stairs", "cellar_room"];
    file.set_interior(key("cellar"), &interior("cellar", &twice))
        .unwrap();
    assert_sound(&file);
    let before = file.clone();
    file.copy_interior(
        key("cellar"),
        key("cellar_two"),
        &[
            (key("cellar_room"), key("cellar_two_room")),
            (key("shared_stairs"), key("cellar_two_stairs")),
        ],
    )
    .unwrap();
    assert_sound(&file);
    assert_eq!(
        file.interior(key("cellar_two")).unwrap(),
        Some(interior(
            "cellar",
            &["cellar_two_room", "cellar_two_stairs", "cellar_two_room"]
        ))
    );
    assert_eq!(
        file.interior_room(key("cellar_two_room")).unwrap(),
        Some(room("cellar_room"))
    );
    assert_eq!(hashed(&file).len(), hashed(&before).len() + 3);
    assert_others_untouched(
        &before,
        &file,
        &[
            key("cellar_two"),
            key("cellar_two_room"),
            key("cellar_two_stairs"),
        ],
    );
    file.remove_interior(key("cellar_two")).unwrap();
    assert_eq!(file, before);
}

#[test]
fn removing_an_interior_leaves_what_it_lists_that_is_not_a_room_of_the_file() {
    // An interior another tool wrote, listing an alarm and a key the file holds nothing under.
    let mut items = hand_built_items();
    items.push((key("attic"), boxed_interior("attic", &["alarm", "nowhere"])));
    let mut file = hand_built_of(items);
    assert_sound(&file);
    file.remove_interior(key("attic")).unwrap();
    assert_eq!(file, hand_built());
}

#[test]
fn a_file_that_does_not_hold_together_is_refused_with_a_sentence() {
    let edit =
        |file: &mut RelFile| refusal(file.add_interior(key("vault"), &interior("vault", &[])));
    let sound = hand_built();
    let lobby = entry(&sound, "lobby");

    let mut by_name = sound.clone();
    by_name.index = RelIndex::Named(vec![RelIndexName {
        name: "lobby".into(),
        offset: lobby.offset,
        length: lobby.length,
    }]);
    let before = by_name.clone();
    let why = edit(&mut by_name);
    assert!(why.contains("keys them by name"), "{why}");
    let why = by_name.interior(key("lobby")).unwrap_err().to_string();
    assert!(why.contains("keys them by name"), "{why}");
    assert_eq!(by_name, before);

    let mut short = sound.clone();
    short.data.truncate(LEAD.len() - 1);
    short.index = RelIndex::Hashed(Vec::new());
    short.hash_offsets.clear();
    short.pack_offsets.clear();
    let before = short.clone();
    let why = edit(&mut short);
    assert!(why.contains("short of the 4 it opens with"), "{why}");
    assert_eq!(short, before);

    // An item the table puts past the data block's end.
    let mut outside = sound.clone();
    let RelIndex::Hashed(index) = &mut outside.index else {
        panic!()
    };
    let at = index
        .iter()
        .position(|entry| entry.hash == key("lobby"))
        .unwrap();
    index[at].length = outside.data.len() as u32;
    let before = outside.clone();
    let why = edit(&mut outside);
    assert!(why.contains("lies outside the data block"), "{why}");
    let why = outside.interior(key("lobby")).unwrap_err().to_string();
    assert!(why.contains("lies outside the data block"), "{why}");
    assert_eq!(outside, before);

    // One key in the item table twice.
    let mut twice = sound.clone();
    let RelIndex::Hashed(index) = &mut twice.index else {
        panic!()
    };
    index.insert(at, lobby);
    let before = twice.clone();
    let why = edit(&mut twice);
    assert!(why.contains("holds two items under"), "{why}");
    assert_eq!(twice, before);

    // An item whose class id is no type's.
    let mut typeless = sound.clone();
    typeless.data[lobby.offset as usize] = u8::MAX;
    let before = typeless.clone();
    let why = edit(&mut typeless);
    assert!(why.contains("has no type"), "{why}");
    let why = typeless.interior(key("lobby")).unwrap_err().to_string();
    assert!(why.contains("has no type"), "{why}");
    assert_eq!(typeless, before);
}
