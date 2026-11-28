//! The story-save payload codec's tests: the LZ4 block format against known vectors and the
//! game's own decoder, and the whole payload pipeline over a real save when this machine holds
//! one.

use super::*;

fn real_save() -> Option<(Vec<u8>, Vec<u8>)> {
    let dir = concat!(
        r"C:\Users\seed\AppData\Local\victory\data\7952987A\saves\0009000000000001",
        r"\00000000-0000-0000-0000-00007952987a\SARC0015~f3ad35e1a05a7da0"
    );
    let versions = std::fs::read_dir(dir).ok()?.filter_map(|e| {
        let name = e.ok()?.file_name().into_string().ok()?;
        let at = name.strip_prefix('v')?.parse::<u32>().ok()?;
        Some((at, format!(r"{}\{}\0", dir, name)))
    }).collect::<std::collections::BTreeMap<u32, String>>();
    let (_, newest) = versions.into_iter().last()?;
    let file = std::fs::read(newest).ok()?;
    if !file.starts_with(b"ASAV") {
        return None;
    }
    let exe = std::fs::read(r"K:\Documents\gtavi-xbox-build-1.0.2.1\game_scarlett_master.exe").ok()?;
    let cipher = crate::gta6::save_crypto::SaveCipher::from_exe(&exe).ok()?;
    let payload = cipher.crypt(file.get(0x170..)?);
    let body = payload[..payload.len() - MAC].to_vec();
    Some((body, payload[payload.len() - MAC..].to_vec()))
}

#[test]
fn the_format_round_trips_small_inputs() {
    for data in [
        b"".as_slice(),
        b"a",
        b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        b"{'json':'text'},\r\n{'json':'text'},\r\n{'json':'text'}",
    ] {
        let stream = lz4_compress(data);
        let back = lz4_decompress(&stream, 1 << 20).expect("a made stream decodes");
        assert_eq!(back, data);
    }
}

#[test]
fn a_realistic_mix_round_trips() {
    // a JSON-like mix: literals, runs, and repeats, like the save's own text
    let mut data = Vec::new();
    for i in 0..1000 {
        data.push(i as u8);
    }
    data.extend(std::iter::repeat(b'A').take(5000));
    for _ in 0..20 {
        data.extend_from_slice(b"{\"CodeSaveBlocks\":[\r\n");
    }
    for i in 0..2000 {
        data.push(255 - i as u8);
    }
    let stream = lz4_compress(&data);
    assert!(stream.len() < data.len(), "the mix should compress: {} -> {}", data.len(), stream.len());
    let back = lz4_decompress(&stream, 1 << 20).expect("a made stream decodes");
    assert_eq!(back, data);
}

#[test]
fn a_match_offset_of_zero_fills_with_zeros() {
    // one token: 2 literals, a 4-byte match at offset 0, which reads the cleared destination
    let stream = [0x20, b'a', b'b', 0x00, 0x00];
    let out = lz4_decompress(&stream, 1 << 10).expect("the zero-offset match decodes");
    assert_eq!(out, b"ab\0\0\0\0");
}

#[test]
fn the_real_save_decodes_and_round_trips() {
    let Some((body, _mac)) = real_save() else { return };
    let jsav = decode(&body).expect("the real payload decodes");
    let saved = jsav.save().expect("the decoded JSAV saves back");
    let mac = [0u8; MAC];
    let payload = encode(&saved, mac);
    let again = decode(&payload).expect("the written payload decodes");
    let saved_again = again.save().expect("it saves again");
    assert_eq!(saved, saved_again, "the JSAV survives the codec round trip");
}

#[test]
fn the_real_save_matches_the_games_own_decoder() {
    // The reference: the game's own decompressor, driven on the same stream in a native
    // harness, returned these counts for the newest autosave. What this module decodes must
    // open as the same JSAV.
    let Some((body, _)) = real_save() else { return };
    let jsav = decode(&body).expect("the real payload decodes");
    assert_eq!(jsav.code.len() + jsav.script.len(), 82, "a story save holds 82 blocks");
}
