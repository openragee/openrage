//! The story-save cipher's tests: the reference vectors the emulated executable gave, the counter
//! walk, and — when the build and a real save are on this machine — the round trip over them.

use super::*;

/// The second reference vector: the block of the counter `00 01 .. 0F`, which the emulated block
/// function of the same build produced alongside the seed's own block.
const REFERENCE_COUNTER_BLOCK: [u8; 16] = [
    0xE6, 0x01, 0xE4, 0x12, 0x42, 0x5A, 0x75, 0xCD, 0x78, 0xD4, 0x9E, 0xF0, 0x3F, 0xD5, 0xDB, 0xF4,
];

/// Where the measured build lies when the test that needs it runs, and the newest autosave it wrote.
const EXE: &str = r"K:\Documents\gtavi-xbox-build-1.0.2.1\game_scarlett_master.exe";
const SAVE: &str = concat!(
    r"C:\Users\seed\AppData\Local\victory\data\7952987A\saves\0009000000000001",
    r"\00000000-0000-0000-0000-00007952987a\SARC0015~f3ad35e1a05a7da0\v39\0"
);

fn cipher() -> Option<SaveCipher> {
    let exe = std::fs::read(EXE).ok()?;
    SaveCipher::from_exe(&exe).ok()
}

#[test]
fn the_seed_block_matches_the_emulated_executable() {
    let Some(cipher) = cipher() else { return };
    assert_eq!(cipher.keystream_block(cipher.seed()), REFERENCE_BLOCK);
}

#[test]
fn the_first_counter_block_matches_the_emulated_executable() {
    let Some(cipher) = cipher() else { return };
    let counter = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    ];
    assert_eq!(cipher.keystream_block(&counter), REFERENCE_COUNTER_BLOCK);
}

#[test]
fn the_counter_carries_from_the_last_byte() {
    let mut short = [0u8; 16];
    short[14] = 0;
    short[15] = 0xFF;
    let mut after = [0u8; 16];
    after[14] = 1;
    after[15] = 0;
    assert_eq!(SaveCipher::next_counter(&short), after);
    let mut counter = [0xFF; 16];
    counter[0] = 0xFD;
    assert_eq!(SaveCipher::next_counter(&counter), [0xFE, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn a_payload_round_trips_through_the_one_operation() {
    let Some(cipher) = cipher() else { return };
    let plain: Vec<u8> = (0..100u32).map(|i| (i * 7 + 3) as u8).collect();
    let crypt = cipher.crypt(&plain);
    assert_ne!(&crypt[..], &plain[..]);
    assert_eq!(cipher.crypt(&crypt), plain);
}

#[test]
fn the_newest_autosave_decrypts_to_a_jsav_head() {
    let (Some(cipher), Ok(save)) = (cipher(), std::fs::read(SAVE)) else { return };
    assert!(save.starts_with(b"ASAV"));
    let plain = cipher.crypt(save.get(0x170..).unwrap());
    assert_eq!(&plain[2..6], b"JSAV");
    assert_eq!(u32::from_le_bytes(plain[6..10].try_into().unwrap()), 3);
}

#[test]
fn the_decrypted_payload_ends_with_a_pkcs7_pad_and_a_32_byte_tag() {
    // Measured on every story save of the profile: the cipher's plaintext is padded to the cipher's
    // 16-byte block with PKCS#7 (the pad value repeated, 1..16 bytes), then followed by the 32-byte
    // MAC the security singleton verifies (`AsavSecurity::Verify`, vtable slot 8 at 0x1428d4530).
    // The reader's cursor over the payload excludes that tag (slot 9 returns 0x20), and its unpad is
    // `FUN_144c11810`: the last byte before the tag states the pad, every pad byte equals it.
    let (Some(cipher), Ok(save)) = (cipher(), std::fs::read(SAVE)) else { return };
    let plain = cipher.crypt(save.get(0x170..).unwrap());
    let body = &plain[..plain.len() - 32];
    let pad = body[body.len() - 1] as usize;
    assert!((1..=16).contains(&pad), "the pad is {pad}, not 1..=16");
    assert!(
        body[body.len() - pad..].iter().all(|byte| *byte as usize == pad),
        "the pad bytes are not the pad value"
    );
}

#[test]
fn the_two_time_pad_shows_in_two_saves_of_one_profile() {
    // The two saves of one profile share the keystream, so their payloads XOR to their plaintexts'
    // XOR: the `JSAV` heads match, and the first bytes are all zero until the saves' own data
    // diverges. This is the signature that made the cipher worth recovering.
    let (Some(cipher), Ok(a), Ok(b)) = (
        cipher(),
        std::fs::read(SAVE),
        std::fs::read(SAVE.replace(r"\SARC0015~f3ad35e1a05a7da0\v35\0", r"\SARC0001~f3a9b5e1a0576e49\v2\0")),
    ) else { return };
    let (pa, pb) = (cipher.crypt(&a[0x170..]), cipher.crypt(&b[0x170..]));
    let zeros = pa
        .iter()
        .zip(pb.iter())
        .take(0x40)
        .filter(|(x, y)| x == y)
        .count();
    assert!(zeros >= 0x10, "the two heads share only {zeros} of the first 0x40 bytes");
}
