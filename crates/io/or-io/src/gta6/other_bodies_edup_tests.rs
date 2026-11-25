use super::*;

/// `EDUP`, `count`, then `rest`.
fn edup(count: u32, rest: &[u8]) -> Vec<u8> {
    let mut out = b"EDUP".to_vec();
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(rest);
    out
}

fn refusal(bytes: &[u8]) -> String {
    EdupHead::open(bytes).expect_err("refused").to_string()
}

#[test]
fn a_counted_head_reads_its_count_and_payload_length_and_no_length_word() {
    let head = EdupHead::open(&edup(33, &[0xA5; 1540])).expect("reads");
    assert_eq!(
        head,
        EdupHead {
            count: 33,
            payload_len: 1540,
            stated_len: None,
        }
    );
    // A counted head checks no width: 228 is 4 x 57, 1,540 is no multiple of 33.
    assert_eq!(EdupHead::open(&edup(4, &[0; 228])).expect("reads").count, 4);
    assert_eq!(EdupHead::open(&edup(7, &[])).expect("reads").payload_len, 0);
}

#[test]
fn a_head_of_count_zero_reads_the_length_word_the_bytes_after_it_match() {
    let mut rest = 16u32.to_le_bytes().to_vec();
    rest.extend_from_slice(&[0x5A; 16]);
    let head = EdupHead::open(&edup(0, &rest)).expect("reads");
    assert_eq!(
        head,
        EdupHead {
            count: 0,
            payload_len: 20,
            stated_len: Some(16),
        }
    );
    let empty = EdupHead::open(&edup(0, &[0; 4])).expect("reads");
    assert_eq!((empty.payload_len, empty.stated_len), (4, Some(0)));
}

#[test]
fn a_head_that_is_short_or_not_edup_is_refused_by_name() {
    for bytes in [&b""[..], b"EDUP", b"EDUP\0\0\0"] {
        let why = refusal(bytes);
        assert!(why.contains("GTA VI EDUP body"), "{why}");
        assert!(why.contains("shorter than its head"), "{why}");
    }
    let why = refusal(b"EDUQ\x01\0\0\0");
    assert!(why.contains("is not `EDUP`"), "{why}");
}

#[test]
fn a_count_of_zero_with_no_length_word_or_a_wrong_one_is_refused() {
    let why = refusal(&edup(0, &[0; 3]));
    assert!(why.contains("leaves no length word"), "{why}");
    let why = refusal(&edup(0, &[]));
    assert!(why.contains("leaves no length word"), "{why}");
    let mut rest = 16u32.to_le_bytes().to_vec();
    rest.extend_from_slice(&[0; 15]);
    let why = refusal(&edup(0, &rest));
    assert!(why.contains("states 16 bytes where 15 follow it"), "{why}");
    rest.extend_from_slice(&[0; 2]);
    let why = refusal(&edup(0, &rest));
    assert!(why.contains("states 16 bytes where 17 follow it"), "{why}");
}

#[test]
fn the_body_the_head_reads_is_the_body_a_scan_names_edup() {
    let bytes = edup(4, &[0; 228]);
    assert_eq!(
        OtherBody::of(&bytes, bytes.len() as u64),
        Some(OtherBody::Edup)
    );
}
