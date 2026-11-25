//! A decrypted GTA VI compiled script opens as its instruction listing instead of its bytes.
//!
//! A native prints by name from the table `openrage-cli` cached from the user's lifted listings,
//! and by address without one; it runs on the load worker, never the UI thread.

use crate::state::document::{DocPayload, TextDocument};
use or_io::gta6::script::compiled::{disasm, Compiled};
use or_io::gta6::script::native_addresses::NativeAddresses;

/// The listing of `bytes` as text, or `None` when they are no compiled program or their code does not decode.
pub(crate) fn script_payload(bytes: &[u8]) -> Option<DocPayload> {
    let program = Compiled::open(bytes).ok()?;
    let natives = or_title::config::config_dir().and_then(|dir| NativeAddresses::cached(&dir));
    let listing = disasm::listing(&program, natives.as_ref()).ok()?;
    Some(DocPayload::Text(TextDocument::decode(
        listing.text.as_bytes(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A compiled program's body: the header the reader checks, then one `return`.
    fn program() -> Vec<u8> {
        use or_io::gta6::script::compiled::{check_copy, HEADER_LEN, OPCODE_MULTIPLIER};
        let mut b = vec![0u8; HEADER_LEN];
        let hash: u32 = 0x1234_5678;
        b[0x10..0x14].copy_from_slice(&hash.to_le_bytes());
        b[0x24..0x28].copy_from_slice(&check_copy(hash).to_le_bytes());
        // The pointer to the code region adds 0x5000_0000 to its body offset.
        b[0xE8..0xF0].copy_from_slice(&(0x5000_0000u64 + HEADER_LEN as u64).to_le_bytes());
        b[0xF8..0xFC].copy_from_slice(&2u32.to_le_bytes());
        let inverse = (1..=u16::MAX)
            .find(|x| x.wrapping_mul(OPCODE_MULTIPLIER) == 1)
            .unwrap_or(0);
        b.extend_from_slice(&0x90u16.wrapping_mul(inverse).to_le_bytes());
        b
    }

    #[test]
    fn a_compiled_script_opens_as_its_listing() {
        let Some(DocPayload::Text(text)) = script_payload(&program()) else {
            panic!("a compiled program opens as text");
        };
        assert!(text.text.to_string().contains("000000  090  return"));
    }

    #[test]
    fn other_bytes_are_left_to_the_fallback() {
        assert!(
            script_payload(&[0u8; 64]).is_none(),
            "too short for a header"
        );
        let mut wrong = program();
        wrong[0x24] ^= 1;
        assert!(
            script_payload(&wrong).is_none(),
            "a check copy that disagrees"
        );
    }
}
