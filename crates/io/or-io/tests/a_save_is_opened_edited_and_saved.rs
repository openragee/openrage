//! A GTA VI save file — story (`JSAV`) or record (`ASAV`) — opened, freely edited and saved back,
//! then reopened to prove the edits stuck and the framing is intact. The story save's blocks are
//! navigated, their fields changed, added and removed, and the record's title and payload are
//! rewritten with a fresh checksum the game would accept.

use or_io::gta6::save::{
    asav_checksum, key_text, known_field_name, open, Asav, AsavHeader, Save, SaveError, SaveKind,
    ASAV_HEADER, JSAV_VERSION,
};

/// Build JSAV bytes by hand: the 16-byte head, one entry per block, then each block's text at a
/// 16-byte-aligned body start.
fn jsav_bytes(code: &[(u32, &str)], script: &[(u32, &str)]) -> Vec<u8> {
    let count = code.len() + script.len();
    let mut out = b"JSAV".to_vec();
    for word in [JSAV_VERSION, code.len() as u32, script.len() as u32] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    let body_start = (16 + count * 20).next_multiple_of(16);
    let mut body = Vec::new();
    let mut entries = Vec::new();
    for (k, (hash, text)) in code.iter().chain(script).enumerate() {
        let glue = match k {
            0 => "{\"blocks\":[\r\n",
            _ => ",\r\n",
        };
        body.extend_from_slice(glue.as_bytes());
        entries.push((*hash, body_start + body.len(), text.len()));
        body.extend_from_slice(text.as_bytes());
    }
    body.extend_from_slice(b"]\r\n}");
    for (hash, offset, size) in entries {
        for word in [hash, 1, 0, offset as u32, size as u32] {
            out.extend_from_slice(&word.to_le_bytes());
        }
    }
    out.resize(body_start, 0);
    out.extend_from_slice(&body);
    out.resize(out.len().next_multiple_of(16), 0);
    out
}

/// Build a minimal ASAV (header + plaintext JSON payload) with a valid checksum.
fn asav_bytes(kind: u32, title: &str, slot: &str, payload: &str) -> Vec<u8> {
    let header = AsavHeader {
        kind,
        word_0c: 1,
        word_10: 0,
        payload_offset: ASAV_HEADER as u32,
        stored_size: 0,
        size: 0,
        time: 1_700_000_000,
        title: title.into(),
        slot: slot.into(),
        checksum: 0,
        word_164: 0,
        size_again: 0,
    };
    let asav = Asav {
        header,
        payload: payload.as_bytes().to_vec(),
    };
    asav.save().expect("the record saves")
}

fn block_json(group: &str, hash: u32, inner: &str) -> String {
    format!("{{\"{}\":{{\"$0x{hash:08x}\":{inner}}}}}", key_text(group))
}

#[test]
fn a_story_save_is_opened_its_blocks_are_edited_and_it_saves_back() {
    let group = "Progress";
    let group_key = key_text(group);

    let block_a = block_json(
        group,
        0xAA,
        "{\"$0x00000001\":42,\"$0x00000002\":[10,20,30],\"$0x00000003\":\"hello\"}",
    );
    let block_b = block_json(group, 0xBB, "{\"$0x00000004\":true,\"$0x00000005\":99.5}");
    let block_c = block_json(group, 0xCC, "{\"$0x00000006\":{\"$0x00000007\":null}}");
    let bytes = jsav_bytes(&[(0xAA, &block_a), (0xBB, &block_b)], &[(0xCC, &block_c)]);

    // Open — the container is detected and the blocks are split.
    assert_eq!(SaveKind::of(&bytes), Some(SaveKind::Story));
    let mut save = match open(&bytes).expect("opens") {
        Save::Story(j) => j,
        _ => panic!("a story save"),
    };
    assert_eq!(save.code.len(), 2);
    assert_eq!(save.script.len(), 1);

    // Navigate — read individual fields by their key path.
    macro_rules! p {
        ($($seg:expr),+ $(,)?) => { &[group_key.as_str(), $($seg),+] };
    }
    let a = save.block_mut(0xAA).expect("block 0xAA");
    assert_eq!(a.get(p!["$0x000000aa", "$0x00000001"]), Some("42"));
    assert_eq!(a.get(p!["$0x000000aa", "$0x00000002", "1"]), Some("20"));
    assert_eq!(a.get(p!["$0x000000aa", "$0x00000003"]), Some("\"hello\""));

    // Edit — change a scalar, an array element and a string.
    a.set(p!["$0x000000aa", "$0x00000001"], "100")
        .expect("set scalar");
    a.set(p!["$0x000000aa", "$0x00000002", "1"], "999")
        .expect("set array element");
    a.set(p!["$0x000000aa", "$0x00000003"], "\"world\"")
        .expect("set string");

    // Verify the edits before saving.
    let a = save.block_mut(0xAA).expect("block 0xAA");
    assert_eq!(a.get(p!["$0x000000aa", "$0x00000001"]), Some("100"));
    assert_eq!(a.get(p!["$0x000000aa", "$0x00000002"]), Some("[10,999,30]"));
    assert_eq!(a.get(p!["$0x000000aa", "$0x00000003"]), Some("\"world\""));

    // Insert — add a member to an object and an element to an array.
    let a = save.block_mut(0xAA).expect("block 0xAA");
    a.insert(p!["$0x000000aa"], Some("$0x000000ff"), "[1,2,3]")
        .expect("insert member");
    a.insert(p!["$0x000000aa", "$0x00000002"], None, "40")
        .expect("insert element");

    let a = save.block_mut(0xAA).expect("block 0xAA");
    assert_eq!(a.get(p!["$0x000000aa", "$0x000000ff"]), Some("[1,2,3]"));
    assert_eq!(a.get(p!["$0x000000aa", "$0x00000002", "3"]), Some("40"));

    // Remove — take out a member and the element just added.
    let a = save.block_mut(0xAA).expect("block 0xAA");
    a.remove(p!["$0x000000aa", "$0x000000ff"])
        .expect("remove member");
    a.remove(p!["$0x000000aa", "$0x00000002", "3"])
        .expect("remove element");
    assert_eq!(a.get(p!["$0x000000aa", "$0x000000ff"]), None);
    assert_eq!(a.get(p!["$0x000000aa", "$0x00000002"]), Some("[10,999,30]"));

    // Edit another block: flip a boolean and change a float.
    let b = save.block_mut(0xBB).expect("block 0xBB");
    b.set(p!["$0x000000bb", "$0x00000004"], "false")
        .expect("set bool");
    b.set(p!["$0x000000bb", "$0x00000005"], "0.125")
        .expect("set float");

    // Edit a script block: replace a null with an object.
    let c = save.block_mut(0xCC).expect("block 0xCC");
    c.set(
        p!["$0x000000cc", "$0x00000006", "$0x00000007"],
        "{\"nested\":true}",
    )
    .expect("set nested object");

    // Save — the bytes are a valid JSAV.
    let written = save.save().expect("saves");
    assert!(written.starts_with(b"JSAV"));
    assert_eq!(written.len() % 16, 0, "padded to 16 bytes");

    // Reopen — every edit is there.
    let reopened = match open(&written).expect("reopens") {
        Save::Story(j) => j,
        _ => panic!("a story save"),
    };
    assert_eq!(reopened.code.len(), 2);
    assert_eq!(reopened.script.len(), 1);

    let a = &reopened.code[0];
    assert_eq!(a.get(p!["$0x000000aa", "$0x00000001"]), Some("100"));
    assert_eq!(a.get(p!["$0x000000aa", "$0x00000002"]), Some("[10,999,30]"));
    assert_eq!(a.get(p!["$0x000000aa", "$0x00000003"]), Some("\"world\""));

    let b = &reopened.code[1];
    assert_eq!(b.get(p!["$0x000000bb", "$0x00000004"]), Some("false"));
    assert_eq!(b.get(p!["$0x000000bb", "$0x00000005"]), Some("0.125"));

    let c = &reopened.script[0];
    assert_eq!(
        c.get(p!["$0x000000cc", "$0x00000006", "$0x00000007"]),
        Some("{\"nested\":true}")
    );

    // The reopened save round-trips to the same bytes: the edit is stable.
    assert_eq!(reopened.save().expect("saves again"), written);
}

#[test]
fn a_settings_record_is_opened_its_payload_is_edited_and_the_checksum_is_valid() {
    let payload = "{\r\n\t\"$v\":2.0,\
        \r\n\t\"$0xb80c6ae8\":{\"$0x3300c23e\":75,\"$0xee190b4e\":50},\
        \r\n\t\"$0x25584db6\":{\"$0x6a510681\":90}\
        \r\n}";
    let bytes = asav_bytes(2, "Settings", "SETTINGS", payload);

    // Open — kind, title, slot and payload are as stated.
    let Save::Record(mut asav) = open(&bytes).expect("opens") else {
        panic!("a record");
    };
    assert_eq!(asav.header.kind, 2);
    assert_eq!(asav.header.title, "Settings");
    assert_eq!(asav.header.slot, "SETTINGS");
    assert_eq!(asav.text().expect("UTF-8"), payload);

    // The known field names in the payload resolve.
    assert_eq!(known_field_name(0xb80c6ae8), Some("audio"));
    assert_eq!(known_field_name(0x3300c23e), Some("musicLevel"));
    assert_eq!(known_field_name(0xee190b4e), Some("sfxLevel"));
    assert_eq!(known_field_name(0x25584db6), Some("camera"));
    assert_eq!(known_field_name(0x6a510681), Some("fov"));
    assert_eq!(key_text("audio"), "$0xb80c6ae8");

    // Edit the payload: change musicLevel from 75 to 100, fov from 90 to 110.
    let new_payload = payload
        .replace("\"$0x3300c23e\":75", "\"$0x3300c23e\":100")
        .replace("\"$0x6a510681\":90", "\"$0x6a510681\":110");
    asav.payload = new_payload.as_bytes().to_vec();

    // Change the title to something with non-ASCII.
    asav.header.title = "Paramètres".into();

    // Save — the writer recomputes the checksum.
    let written = asav.save().expect("saves");
    assert!(written.starts_with(b"ASAV"));

    // The checksum in the file is the joaat of the header with +0x160 zeroed.
    let header: [u8; ASAV_HEADER] = written[..ASAV_HEADER].try_into().unwrap();
    let expected = asav_checksum(&header);
    let stored = u32::from_le_bytes(written[0x160..0x164].try_into().unwrap());
    assert_eq!(stored, expected, "the checksum the writer stamps is valid");

    // Reopen — the edits are there and the checksum passes.
    let Save::Record(reopened) = open(&written).expect("reopens") else {
        panic!("a record");
    };
    assert_eq!(reopened.header.title, "Paramètres");
    let text = reopened.text().expect("UTF-8");
    assert!(text.contains("\"$0x3300c23e\":100"), "musicLevel edited");
    assert!(text.contains("\"$0x6a510681\":110"), "fov edited");
    assert!(text.contains("\"$0xee190b4e\":50"), "sfxLevel untouched");

    // A second save round-trips to the same bytes.
    assert_eq!(reopened.save().expect("saves again"), written);
}

#[test]
fn an_edited_record_whose_payload_grew_still_carries_a_valid_checksum() {
    let payload = "{\"$v\":1}";
    let bytes = asav_bytes(3, "Profile", "PROFILE", payload);
    let Save::Record(mut asav) = open(&bytes).expect("opens") else {
        panic!("a record");
    };

    // A much larger payload: the size fields in the header change, but the checksum covers only
    // the header, so the game still accepts the file.
    let big = format!(
        "{{\"$v\":1,\"$0x00000001\":{}}}",
        "\"A\"".repeat(5000).replace("\"\"", "\",\"")
    );
    asav.payload = big.as_bytes().to_vec();
    let written = asav.save().expect("saves");

    let Save::Record(reopened) = open(&written).expect("reopens") else {
        panic!("a record");
    };
    let total = written.len() as u32;
    assert_eq!(
        reopened.header.size, total,
        "the header states the new size"
    );
    assert_eq!(reopened.header.size_again, u64::from(total));
    assert_eq!(reopened.text().expect("UTF-8"), big.as_str());
}

#[test]
fn a_story_save_refuses_bad_json_and_a_missing_path() {
    let block = block_json("G", 1, "{\"$0x00000001\":42}");
    let bytes = jsav_bytes(&[(1, &block)], &[]);
    let mut save = match open(&bytes).expect("opens") {
        Save::Story(j) => j,
        _ => panic!("a story save"),
    };
    let group = key_text("G");
    let b = save.block_mut(1).expect("the block");

    // Setting a value to invalid JSON is refused.
    assert!(matches!(
        b.set(&[group.as_str(), "$0x00000001", "$0x00000001"], "{broken"),
        Err(SaveError::NotJson(_))
    ));

    // A path that leads nowhere is refused.
    assert!(matches!(
        b.set(&[group.as_str(), "$0x00000001", "$0x000000ff"], "1"),
        Err(SaveError::NoValue { .. })
    ));

    // The block is unchanged by the refused edits.
    assert_eq!(
        b.get(&[group.as_str(), "$0x00000001", "$0x00000001"]),
        Some("42")
    );
}

#[test]
fn a_tampered_asav_header_is_refused() {
    let bytes = asav_bytes(2, "Test", "SLOT", "{\"$v\":1}");
    // Flip one bit in the title area: the checksum now disagrees.
    let mut tampered = bytes.clone();
    tampered[0x30] ^= 1;
    assert!(
        matches!(open(&tampered), Err(SaveError::Checksum { .. })),
        "a flipped bit in the header is caught by the checksum"
    );
}
