use super::*;

#[test]
fn an_unreadable_rpf9_table_never_falls_back_to_a_scan() {
    let dir = tempfile::tempdir().expect("scratch directory");
    let path = dir.path().join("unreadable.rpf");
    let bytes = crate::state::opening_an_rpf9_archive::rpf9_head(1, 0x1A0);
    std::fs::write(&path, bytes).expect("write a header with no readable table");
    let result = parse_payload_from_path(
        &path,
        DocumentKind::Rpf,
        Arc::new(or_io::container::NoOpCryptoProvider),
    );
    assert!(
        result.is_err(),
        "opening an unreadable table started a scan"
    );
}
