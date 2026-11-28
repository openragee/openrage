use super::*;

#[test]
fn an_entry_that_starts_as_a_program_and_does_not_read_is_counted_unread() {
    let mut truncated = vec![0u8; 8];
    truncated.extend_from_slice(&[0xAB; 8]);
    assert_eq!(kept(&truncated), Kept::Unread);
    assert_eq!(kept(&[0xAB; 16]), Kept::Other);
}
