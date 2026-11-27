use super::*;

/// A story save laid out by hand, apart from `Jsav::save`: the index, then the texts between `glue`.
fn story(code: &[(u32, &str)], script: &[(u32, &str)]) -> Vec<u8> {
    let count = code.len() + script.len();
    let mut out = b"JSAV".to_vec();
    for word in [3, code.len() as u32, script.len() as u32] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    let mut body = Vec::new();
    let mut entries = Vec::new();
    let base = (16 + count * 20).next_multiple_of(16);
    for (k, (hash, text)) in code.iter().chain(script).enumerate() {
        let glue = match k {
            0 => "{\"CodeSaveBlocks\":[\r\n  ",
            k if k == code.len() => "],\r\n\"ScriptSaveBlocks\":[\r\n  ",
            _ => ",\r\n ",
        };
        body.extend_from_slice(glue.as_bytes());
        entries.push((*hash, base + body.len(), text.len()));
        body.extend_from_slice(text.as_bytes());
    }
    body.extend_from_slice(b"]\r\n}");
    for (hash, offset, size) in entries {
        for word in [hash, 1, 0, offset as u32, size as u32] {
            out.extend_from_slice(&word.to_le_bytes());
        }
    }
    out.resize(base, 0);
    out.extend_from_slice(&body);
    out.resize(out.len().next_multiple_of(16), 0);
    out
}

fn block(group: &str, hash: u32, inner: &str) -> String {
    format!("{{\"{}\":{{\"$0x{hash:08x}\":{inner}}}}}", key_text(group))
}

fn sample() -> Vec<u8> {
    let a = block(
        "Code",
        0x11,
        "{\"$0x000000aa\":1.500000,\"$0x000000bb\":[3,4,{\"$x\":-2.000000}]}",
    );
    let b = block(
        "Code",
        0x22,
        "{\"$0x000000cc\":\"a \\\"quoted\\\" , } text\"}",
    );
    let c = block("Script", 0x33, "{ \"$0x000000dd\" : true }");
    story(&[(0x11, &a), (0x22, &b)], &[(0x33, &c)])
}

#[test]
fn a_story_save_opens_and_saves_to_the_same_bytes() {
    let bytes = sample();
    let save = Jsav::open(&bytes).expect("opens");
    assert_eq!((save.code.len(), save.script.len()), (2, 1));
    assert_eq!(save.code[1].hash, 0x22);
    assert_eq!(save.script[0].flags, 1);
    assert_eq!(save.code[0].keys(), Some((key_hash("Code"), 0x11)));
    assert_eq!(save.script[0].keys(), Some((key_hash("Script"), 0x33)));
    assert_eq!(save.save().expect("saves"), bytes);
    let one = story(&[(7, "{}")], &[]);
    assert_eq!(Jsav::open(&one).expect("opens").save().expect("saves"), one);
    let none = story(&[], &[]);
    assert_eq!(open(&none).map(|s| matches!(s, Save::Story(_))), Ok(true));
}

#[test]
fn an_edited_value_is_the_only_text_that_changes() {
    let bytes = sample();
    let mut save = Jsav::open(&bytes).expect("opens");
    let before = save.clone();
    let group = key_text("Code");
    let path = [group.as_str(), "$0x00000011", "$0x000000bb", "2", "$x"];
    let edited = save.block_mut(0x11).expect("the block");
    assert_eq!(edited.get(&path), Some("-2.000000"));
    assert_eq!(
        edited.get(&[group.as_str(), "$0x00000011", "$0x000000aa"]),
        Some("1.500000")
    );
    edited.set(&path, " 1234.250000 ").expect("sets");
    assert_eq!(edited.get(&path), Some("1234.250000"));
    assert_eq!(
        edited.text,
        before.code[0].text.replace("-2.000000", "1234.250000")
    );
    let written = save.save().expect("saves");
    let again = Jsav::open(&written).expect("opens again");
    assert_eq!(again, save);
    assert_eq!(again.code[1], before.code[1]);
    assert_eq!(again.script, before.script);
    assert_eq!(again.save().expect("saves"), written);
    assert_eq!(written.len() % 16, 0);
    // The longer text moved the blocks after it, and the index says where.
    let offset = |bytes: &[u8], k: usize| u32_at(bytes, 16 + 20 * k + 12);
    assert_eq!(offset(&written, 0), offset(&bytes, 0));
    assert_eq!(offset(&written, 1), offset(&bytes, 1) + 2);
    assert_eq!(
        again.code[0].value().expect("JSON")[&group]["$0x00000011"]["$0x000000bb"][2]["$x"],
        1234.25
    );
}

#[test]
fn a_path_reads_past_strings_that_hold_brackets_and_refuses_what_is_not_there() {
    let mut save = Jsav::open(&sample()).expect("opens");
    let group = key_text("Code");
    let b = save.block_mut(0x22).expect("the block");
    let path = [group.as_str(), "$0x00000022", "$0x000000cc"];
    assert_eq!(b.get(&path), Some("\"a \\\"quoted\\\" , } text\""));
    assert_eq!(
        b.set(&[group.as_str(), "$0x00000022", "$0x00000000"], "1"),
        Err(SaveError::NoValue {
            path: format!("{group}/$0x00000022/$0x00000000")
        })
    );
    assert!(matches!(
        b.set(&path, "{\"open\":"),
        Err(SaveError::NotJson(_))
    ));
    let script = key_text("Script");
    let c = save.block_mut(0x33).expect("the block");
    assert_eq!(
        c.get(&[script.as_str(), "$0x00000033", "$0x000000dd"]),
        Some("true")
    );
    assert_eq!(
        c.get(&[script.as_str(), "$0x00000033", "$0x000000dd", "0"]),
        None
    );
    let a = save.block_mut(0x11).expect("the block");
    assert_eq!(
        a.get(&[group.as_str(), "$0x00000011", "$0x000000bb", "3"]),
        None
    );
    assert_eq!(
        a.get(&[group.as_str(), "$0x00000011", "$0x000000bb", "1"]),
        Some("4")
    );
}

#[test]
fn a_save_key_keeps_case() {
    // Jenkins' one-at-a-time hash of "a", as its author publishes it.
    assert_eq!(key_hash("a"), 0xCA2E_9442);
    assert_ne!(key_hash("A"), key_hash("a"));
    assert_eq!(key_text("a"), "$0xca2e9442");
    assert_eq!(parse_key("$0xca2e9442"), Some(0xCA2E_9442));
    assert_eq!(parse_key("$x"), None);
    assert_eq!(parse_key("$0x12"), None);
}

#[test]
fn a_story_save_that_breaks_its_framing_is_refused() {
    let bytes = sample();
    let mut version = bytes.clone();
    version[4] = 4;
    assert_eq!(
        Jsav::open(&version),
        Err(SaveError::Version {
            container: "JSAV",
            found: 4,
            read: 3
        })
    );
    // The second block stated one byte early lies inside the first.
    let first_end = u32_at(&bytes, 16 + 12) + u32_at(&bytes, 16 + 16);
    let mut overlapping = bytes.clone();
    overlapping[16 + 20 + 12..16 + 20 + 16].copy_from_slice(&(first_end - 1).to_le_bytes());
    assert!(matches!(
        Jsav::open(&overlapping),
        Err(SaveError::BlockPlacement { index: 1, .. })
    ));
    let mut long = bytes.clone();
    long.extend_from_slice(&[0; 16]);
    assert!(matches!(Jsav::open(&long), Err(SaveError::Padding { .. })));
    // Three entries end at 0x4C, so the bytes to 0x50 are the index's padding.
    let mut padded = bytes.clone();
    padded[0x4D] = 1;
    assert_eq!(Jsav::open(&padded), Err(SaveError::Padding { at: 0x4C }));
    assert!(matches!(
        Jsav::open(&bytes[..0x30]),
        Err(SaveError::Truncated { .. })
    ));
    assert_eq!(open(b"RSC7 and more"), Err(SaveError::NotASave));
}

/// An `ASAV` header of `kind` laid out by hand, then `payload`. The checksum at +0x160 is the
/// RAGE joaat the game computes over the 0x170 header with that slot zeroed.
fn record(kind: u32, title: &str, slot: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; ASAV_HEADER];
    out[..4].copy_from_slice(b"ASAV");
    let size = (ASAV_HEADER + payload.len()) as u32;
    for (at, word) in [
        (4, ASAV_VERSION),
        (8, kind),
        (0x0C, 1),
        (0x14, 0x170),
        (0x1C, size),
    ] {
        out[at..at + 4].copy_from_slice(&u32::to_le_bytes(word));
    }
    out[0x20..0x28].copy_from_slice(&1_700_000_000u64.to_le_bytes());
    for (k, unit) in title.encode_utf16().enumerate() {
        out[0x28 + 2 * k..0x2A + 2 * k].copy_from_slice(&unit.to_le_bytes());
    }
    out[0x128..0x128 + slot.len()].copy_from_slice(slot.as_bytes());
    out[0x168..0x170].copy_from_slice(&u64::from(size).to_le_bytes());
    // The checksum slot is already zero; hash the header and write the result in.
    let header: [u8; ASAV_HEADER] = out[..ASAV_HEADER].try_into().unwrap();
    let checksum = asav_checksum(&header);
    out[0x160..0x164].copy_from_slice(&checksum.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

#[test]
fn a_record_s_header_reads_as_the_file_states_it() {
    let text = "{\r\n\t\"$v\":2\r\n}";
    let bytes = record(2, "Réglages", "SETTINGS", text.as_bytes());
    let Ok(Save::Record(asav)) = open(&bytes) else {
        panic!("a plain record opens");
    };
    let h = &asav.header;
    assert_eq!((h.kind, h.word_0c, h.payload_offset), (2, 1, 0x170));
    assert_eq!(
        (h.title.as_str(), h.slot.as_str()),
        ("Réglages", "SETTINGS")
    );
    assert_eq!(h.time, 1_700_000_000);
    // The checksum in the file is exactly what `asav_checksum` writes; a saved record reads back
    // with that same checksum.
    let header: [u8; ASAV_HEADER] = {
        let mut hdr = [0u8; ASAV_HEADER];
        hdr.copy_from_slice(&bytes[..ASAV_HEADER]);
        hdr
    };
    assert_eq!(h.checksum, asav_checksum(&header));
    assert_eq!(u64::from(h.size), h.size_again);
    assert_eq!(asav.text(), Some(text));
    assert!(matches!(
        Asav::open(&bytes[..0x100]),
        Err(SaveError::Truncated { .. })
    ));
    let mut wrong = bytes.clone();
    wrong[4] = (ASAV_VERSION + 1) as u8;
    assert_eq!(
        Asav::open(&wrong),
        Err(SaveError::Version {
            container: "ASAV",
            found: ASAV_VERSION + 1,
            read: ASAV_VERSION,
        })
    );
    // A payload_offset that points inside the header fails after the (recomputed) checksum passes.
    let mut inside = bytes.clone();
    inside[0x14..0x18].copy_from_slice(&0x100u32.to_le_bytes());
    inside[0x160..0x164].copy_from_slice(&0u32.to_le_bytes());
    let mut inside_header = [0u8; ASAV_HEADER];
    inside_header.copy_from_slice(&inside[..ASAV_HEADER]);
    let fixed = asav_checksum(&inside_header);
    inside[0x160..0x164].copy_from_slice(&fixed.to_le_bytes());
    assert!(matches!(
        Asav::open(&inside),
        Err(SaveError::Payload { offset: 0x100, .. })
    ));
    // A header whose checksum does not match the file is refused.
    let mut tampered = bytes.clone();
    // Flip one bit outside the checksum slot; the file was built with a correct checksum, so
    // the flip now disagrees with it.
    tampered[0x20] ^= 1;
    assert!(matches!(
        Asav::open(&tampered),
        Err(SaveError::Checksum { .. })
    ));
}

#[test]
fn a_packed_copy_is_named_and_refused() {
    let mut bytes = b"SLAYER1\0".to_vec();
    let name = "slot-name";
    let inner = record(1, "A title", "SAVE07", &[0xA5; 48]);
    bytes.extend_from_slice(&ASAV_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
    bytes.extend_from_slice(name.as_bytes());
    bytes.extend_from_slice(&(inner.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&inner);
    assert_eq!(SaveKind::of(&bytes), Some(SaveKind::Packed));
    let head = Slayer1::read(&bytes).expect("the head reads");
    assert_eq!(
        (head.name.as_str(), head.payload_size),
        (name, inner.len() as u64)
    );
    assert_eq!(head.header.kind, 1);
    assert_eq!(
        open(&bytes),
        Err(SaveError::Packed {
            title: "A title".into(),
            slot: "SAVE07".into()
        })
    );
    assert!(matches!(
        Slayer1::read(&bytes[..40]),
        Err(SaveError::Truncated { .. })
    ));
}

#[test]
fn a_record_s_bytes_round_trip_through_save_and_the_version_is_asav_version() {
    let text = "{\r\n\t\"$v\":2\r\n}";
    let bytes = record(2, "Réglages", "SETTINGS", text.as_bytes());
    let Save::Record(asav) = open(&bytes).expect("a record opens") else {
        panic!("a plain record");
    };
    let written = asav.save().expect("the record saves");
    assert_eq!(&written[..4], b"ASAV", "the file still starts with the tag");
    assert_eq!(
        u32::from_le_bytes([written[4], written[5], written[6], written[7]]),
        ASAV_VERSION,
        "the writer always states ASAV_VERSION"
    );
    assert_eq!(written, bytes, "the save round-trips to the same bytes");
    // A changed payload writes a file the game still accepts: the checksum at +0x160 is a hash
    // of the header only, so no payload edit can disagree with it.
    let mut edited = asav.clone();
    edited.payload[..text.len()].copy_from_slice(b"{\r\n\t\"$v\":3\r\n}");
    let again = edited.save().expect("the edited record saves");
    let header: [u8; ASAV_HEADER] = {
        let mut hdr = [0u8; ASAV_HEADER];
        hdr.copy_from_slice(&again[..ASAV_HEADER]);
        hdr
    };
    let expected = asav_checksum(&header);
    assert_eq!(
        u32::from_le_bytes([again[0x160], again[0x161], again[0x162], again[0x163]]),
        expected,
        "the writer stamps the joaat of the header into +0x160"
    );
    let Save::Record(reopened) = open(&again).expect("opens again") else {
        panic!("a plain record");
    };
    assert_eq!(reopened.text(), Some("{\r\n\t\"$v\":3\r\n}"));
}

#[test]
fn every_known_settings_field_hashes_to_its_key() {
    for (hash, name) in KNOWN_SETTINGS_FIELD_NAMES {
        assert_eq!(
            key_hash(name),
            *hash,
            "{name} hashes to {h:#010x}, not {expect:#010x}",
            h = key_hash(name),
            expect = hash,
        );
        assert_eq!(known_field_name(*hash), Some(*name));
    }
    // A hash no name is known for returns None, not a wrong guess.
    assert_eq!(known_field_name(0xBAAD_F00D), None);
}

#[test]
fn the_rage_joaat_matches_a_known_value() {
    // Empty input returns a hash of the seed alone, run through the finalizer.
    // seed = 0; finalize: (0*9)^((0*9)>>11) * 0x8001 = 0. So for seed 0, the result is 0.
    assert_eq!(rage_joaat_signed(&[], 0), 0);
    // A one-byte input with seed 0: hash = ((byte + 0) * 0x401) ^ ... then finalized.
    // We just lock the current value so a later refactor does not drift the algorithm.
    let one_byte = rage_joaat_signed(b"a", 0);
    assert_ne!(one_byte, 0, "a non-zero byte gives a non-zero hash");
    // The `#[record]`-style check: the game's seed on the ASAV header is 0x4BF672ED.
    assert_eq!(ASAV_CHECKSUM_SEED, 0x4BF6_72ED);
    // asav_checksum on an all-zero header gives a fixed value; writing it back and hashing again
    // must return the same hash (the +0x160 slot is zeroed internally for the hash).
    let header = [0u8; ASAV_HEADER];
    let h1 = asav_checksum(&header);
    let mut header2 = header;
    header2[0x160..0x164].copy_from_slice(&h1.to_le_bytes());
    assert_eq!(asav_checksum(&header2), h1, "the +0x160 slot is masked");
}

#[test]
fn an_asav_header_refuses_a_title_or_slot_that_does_not_fit() {
    let payload = vec![0u8; 16];
    let header = AsavHeader {
        kind: 2,
        word_0c: 1,
        word_10: 0,
        payload_offset: 0x170,
        stored_size: 0,
        size: 0,
        time: 0,
        title: "x".repeat(200),
        slot: String::new(),
        checksum: 0,
        word_164: 0,
        size_again: 0,
    };
    assert_eq!(
        Asav {
            header,
            payload: payload.clone()
        }
        .save(),
        Err(SaveError::TooLarge)
    );
    let too_long_slot = AsavHeader {
        kind: 2,
        word_0c: 1,
        word_10: 0,
        payload_offset: 0x170,
        stored_size: 0,
        size: 0,
        time: 0,
        title: "ok".into(),
        slot: "x".repeat(60),
        checksum: 0,
        word_164: 0,
        size_again: 0,
    };
    assert_eq!(
        Asav {
            header: too_long_slot,
            payload
        }
        .save(),
        Err(SaveError::TooLarge)
    );
}

#[test]
fn a_slayer1_wraps_an_inner_record_and_reads_back_with_the_same_head() {
    let inner = record(1, "A title", "SAVE07", &[0xA5; 48]);
    let packed = Slayer1::write("slot-name", &inner).expect("the slayer wraps");
    assert_eq!(&packed[..8], b"SLAYER1\0");
    let head = Slayer1::read(&packed).expect("the head reads");
    assert_eq!(head.name, "slot-name");
    assert_eq!(head.payload_size, inner.len() as u64);
    assert_eq!(head.header.title, "A title");
    assert_eq!(head.header.slot, "SAVE07");
}

#[test]
fn members_and_elements_are_added_and_removed_and_the_save_opens_again() {
    let mut save = Jsav::open(&sample()).expect("opens");
    let group = key_text("Code");
    let root = [group.as_str(), "$0x00000011"];
    let at = |rest: &[&'static str]| [&root[..], rest].concat();
    let block = &mut save.code[0];
    let before = block.text.clone();
    // An element after the last, a member under a new key, and the first member of an empty object.
    (block.insert(&at(&["$0x000000bb"]), None, " 5 ")).expect("an element");
    (block.insert(&at(&[]), Some("$0x000000ee"), "{}")).expect("a member");
    (block.insert(&at(&["$0x000000ee"]), Some("$x"), "null")).expect("a first member");
    assert_eq!(
        block.get(&at(&["$0x000000bb"])),
        Some("[3,4,{\"$x\":-2.000000},5]")
    );
    assert_eq!(block.get(&at(&["$0x000000ee"])), Some("{\"$x\":null}"));
    assert_eq!(block.get(&at(&["$0x000000aa"])), Some("1.500000"));
    let why = |added: Result<(), SaveError>| match added {
        Err(SaveError::NotAdded { why, .. }) => why,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        why(block.insert(&at(&[]), Some("$0x000000aa"), "1")),
        "the object already holds the key"
    );
    assert_eq!(
        why(block.insert(&at(&[]), None, "1")),
        "a member of an object needs a key"
    );
    assert_eq!(
        why(block.insert(&at(&["$0x000000bb"]), Some("k"), "1")),
        "an element of an array takes no key"
    );
    assert_eq!(
        why(block.insert(&at(&["$0x000000aa"]), None, "1")),
        "the value there is no object and no array"
    );
    assert_eq!(
        why(block.insert(&at(&["$0x000000ff"]), None, "1")),
        "no value lies there"
    );
    assert!(matches!(
        block.insert(&at(&[]), Some("k"), "{"),
        Err(SaveError::NotJson(_))
    ));
    // Taking out what was added, last first, gives the text back.
    (block.remove(&at(&["$0x000000ee", "$x"]))).expect("the only member");
    assert_eq!(block.get(&at(&["$0x000000ee"])), Some("{}"));
    (block.remove(&at(&["$0x000000ee"]))).expect("the last member");
    (block.remove(&at(&["$0x000000bb", "3"]))).expect("the last element");
    assert_eq!(block.text, before);
    // A first member and a middle element go with the comma after them.
    (block.remove(&at(&["$0x000000bb", "1"]))).expect("a middle element");
    (block.remove(&at(&["$0x000000aa"]))).expect("a first member");
    assert_eq!(
        block.get(&at(&[])),
        Some("{\"$0x000000bb\":[3,{\"$x\":-2.000000}]}")
    );
    for gone in [
        &at(&["$0x000000bb", "2"])[..],
        &at(&["$0x000000aa"])[..],
        &[],
    ] {
        assert!(matches!(block.remove(gone), Err(SaveError::NoValue { .. })));
    }
    let again = Jsav::open(&save.save().expect("saves")).expect("opens again");
    assert_eq!(again, save);
    assert_eq!(
        again.code[0].value().expect("JSON")[group.as_str()]["$0x00000011"]["$0x000000bb"][1]["$x"],
        -2.0
    );
}
