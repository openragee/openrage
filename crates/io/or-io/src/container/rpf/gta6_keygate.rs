//! Exe-gated masking for the GTA VI key snapshot: the plaintext `ORGTAVIK` container is XORed with a
//! keystream keyed on bytes only the game executable carries, so the masked file on disk is inert
//! without the user's own executable. This mirrors GTA V's `magic.dat`, which the AES key extracted
//! from the executable unlocks; here the key is the white-box cipher's own table region, located by
//! the image layout the RPF9 TOC reader relies on: the PE layout on Xbox, the SELF container's
//! segment table on PlayStation 5.
//!
//! ⚠ Reversible only with the executable, so a masked snapshot can be committed where the plaintext
//! one cannot (D18). The masking algorithm and the image-layout constants are committable; the
//! executable's bytes and the plaintext snapshot are not. Each masked snapshot is in the tree once,
//! where `bundled-keys` embeds it (`the_key_snapshot_is_in_the_tree_once`).

use sha2::{Digest, Sha256};

use super::gta6_image::{is_self, self_map};
use crate::error::{IoError, Result};

/// The masked snapshot for an Xbox build that `bundled-keys` embeds: inert until the user's own
/// executable keys it (D18).
#[cfg(feature = "bundled-keys")]
pub const BUNDLED_SNAPSHOT: &[u8] = include_bytes!("../../../data/gtavi_xboxsx_magic.dat");

/// The masked snapshot for a PlayStation 5 build that `bundled-keys` embeds, the same rule as
/// [`BUNDLED_SNAPSHOT`]'s.
#[cfg(feature = "bundled-keys")]
pub const BUNDLED_PS5_SNAPSHOT: &[u8] = include_bytes!("../../../data/gtavi_ps5_magic.dat");

/// The marker a masked snapshot opens with; a container that begins with it needs the executable.
pub const MASKED_MAGIC: &[u8; 8] = b"ORGTAVIM";

/// The plaintext container's magic, recovered after unmasking.
const PLAIN_MAGIC: &[u8; 8] = b"ORGTAVIK";

/// The virtual address of the cipher's shared tables in the Xbox executable: the 16 input byte
/// tables (`0x1000`) immediately followed by the 16 round word tables (`0x4000`), one contiguous
/// `0x5000`-byte region whose bytes are the cipher's secret and appear in no public source.
const PE_GATE_REGION_VA: u64 = 0x1_4603_6f30;
/// The same region's virtual address in a PlayStation 5 build's `eboot.bin`.
const SELF_GATE_REGION_VA: u64 = 0x09B3_3D80;
const GATE_REGION_LEN: usize = 0x5000;

/// A 32-byte mask key derived from the executable's cipher-table region — the Xbox region of a PE,
/// the PlayStation 5 region of a SELF — or, when that region cannot be mapped (a truncated or
/// foreign input), from the whole input, so a wrong executable yields a different but still
/// deterministic key.
pub fn gate_key(exe: &[u8]) -> [u8; 32] {
    let region = if is_self(exe) {
        self_map(exe, SELF_GATE_REGION_VA, GATE_REGION_LEN)
    } else {
        pe_read(exe, PE_GATE_REGION_VA, GATE_REGION_LEN)
    };
    match region {
        Some(region) => Sha256::digest(region).into(),
        None => Sha256::digest(exe).into(),
    }
}

/// Mask `plain` with the keystream keyed on `exe`, returning a [`MASKED_MAGIC`] container that is
/// inert without the executable that keys it.
pub fn encode_masked(plain: &[u8], exe: &[u8]) -> Vec<u8> {
    let key = gate_key(exe);
    let mut out = Vec::with_capacity(MASKED_MAGIC.len() + plain.len());
    out.extend_from_slice(MASKED_MAGIC);
    xor_keystream(&key, plain, &mut out);
    out
}

/// Recover the plaintext `ORGTAVIK` snapshot from a [`MASKED_MAGIC`] container, unmasking with the
/// keystream keyed on `exe`. Refused when the input is not a masked container, or when `exe` does not
/// key it, which leaves the unmasked bytes without the plaintext magic.
pub fn decode_masked(masked: &[u8], exe: &[u8]) -> Result<Vec<u8>> {
    let payload = masked
        .strip_prefix(MASKED_MAGIC)
        .ok_or_else(|| masked_bad("it does not open with the ORGTAVIM marker"))?;
    let key = gate_key(exe);
    let mut plain = Vec::with_capacity(payload.len());
    xor_keystream(&key, payload, &mut plain);
    if !plain.starts_with(PLAIN_MAGIC) {
        return Err(masked_bad(
            "the unmasked bytes do not open with ORGTAVIK; this is not the executable that keyed it",
        ));
    }
    Ok(plain)
}

/// Whether `bytes` is a masked snapshot container that [`decode_masked`] opens.
pub fn is_masked(bytes: &[u8]) -> bool {
    bytes.starts_with(MASKED_MAGIC)
}

fn masked_bad(why: &str) -> IoError {
    IoError::Invalid {
        what: "a masked GTA VI key snapshot".into(),
        why: why.into(),
    }
}

/// XOR `input` with the SHA-256 counter keystream of `key`, appending to `out`: block `i` is
/// `SHA256(key || i_le_u64)`, blocks concatenated and truncated to `input.len()`.
fn xor_keystream(key: &[u8; 32], input: &[u8], out: &mut Vec<u8>) {
    for (i, chunk) in input.chunks(32).enumerate() {
        let mut hasher = Sha256::new();
        hasher.update(key);
        hasher.update((i as u64).to_le_bytes());
        let block: [u8; 32] = hasher.finalize().into();
        for (b, k) in chunk.iter().zip(block.iter()) {
            out.push(b ^ k);
        }
    }
}

/// Read `len` bytes at virtual address `va` from a PE image's raw bytes, mapping the address through
/// the section table as the loader would. `None` on any malformed header or an address no section
/// covers. The arithmetic mirrors the reference TOC reader's PE probe.
fn pe_read(exe: &[u8], va: u64, len: usize) -> Option<&[u8]> {
    let pe = u32_at(exe, 60)? as usize;
    let section_count = u16_at(exe, pe.checked_add(6)?)? as usize;
    let optional = u16_at(exe, pe.checked_add(20)?)? as usize;
    let base = u64_at(exe, pe.checked_add(48)?)?;
    let rva = va.checked_sub(base)?;
    let table = pe.checked_add(24)?.checked_add(optional)?;
    let len_u64 = len as u64;
    for i in 0..section_count {
        let sh = table.checked_add(i.checked_mul(40)?)?;
        let start = u64::from(u32_at(exe, sh.checked_add(12)?)?);
        let raw_size = u64::from(u32_at(exe, sh.checked_add(16)?)?);
        let raw_ptr = u64::from(u32_at(exe, sh.checked_add(20)?)?);
        let end = start.checked_add(raw_size)?;
        if start <= rva && rva.checked_add(len_u64)? <= end {
            let file_off = raw_ptr.checked_add(rva - start)?;
            let begin = usize::try_from(file_off).ok()?;
            return exe.get(begin..begin.checked_add(len)?);
        }
    }
    None
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    bytes
        .get(at..at.checked_add(2)?)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at.checked_add(4)?)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    bytes
        .get(at..at.checked_add(8)?)
        .map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal PE64 image with one section mapping `[rva, rva + raw_size)` to the file bytes at
    /// `raw_ptr`, for exercising [`pe_read`] without a 100 MB buffer.
    fn minimal_pe() -> (Vec<u8>, u64) {
        const BASE: u64 = 0x1_4000_0000;
        const PE: usize = 0x80;
        const OPTIONAL: usize = 0xF0;
        const RVA: u32 = 0x1000;
        const RAW_PTR: u32 = 0x400;
        const RAW_SIZE: u32 = 0x400;
        let mut buf = vec![0u8; 0x800];
        buf[60..64].copy_from_slice(&(PE as u32).to_le_bytes());
        buf[PE..PE + 4].copy_from_slice(b"PE\0\0");
        buf[PE + 6..PE + 8].copy_from_slice(&1u16.to_le_bytes()); // one section
        buf[PE + 20..PE + 22].copy_from_slice(&(OPTIONAL as u16).to_le_bytes());
        buf[PE + 48..PE + 56].copy_from_slice(&BASE.to_le_bytes());
        let sh = PE + 24 + OPTIONAL;
        buf[sh + 12..sh + 16].copy_from_slice(&RVA.to_le_bytes());
        buf[sh + 16..sh + 20].copy_from_slice(&RAW_SIZE.to_le_bytes());
        buf[sh + 20..sh + 24].copy_from_slice(&RAW_PTR.to_le_bytes());
        // Distinct bytes inside the mapped section, at file 0x500 (rva 0x1100).
        for (i, slot) in buf[0x500..0x520].iter_mut().enumerate() {
            *slot = 0x40 + i as u8;
        }
        (buf, BASE + 0x1100)
    }

    /// A PE whose one section maps the gate's own cipher-table region ([`PE_GATE_REGION_VA`], length
    /// [`GATE_REGION_LEN`]) to a few KiB of file bytes, so [`gate_key`] can be shown to key on the
    /// region rather than the whole file — without a 100 MB buffer. The region's file offset is
    /// `0x400`, filled with distinct bytes.
    fn region_pe() -> Vec<u8> {
        const BASE: u64 = 0x1_4000_0000;
        const PE: usize = 0x80;
        const OPTIONAL: usize = 0xF0;
        const RAW_PTR: usize = 0x400;
        let rva = (PE_GATE_REGION_VA - BASE) as u32;
        let mut buf = vec![0u8; RAW_PTR + GATE_REGION_LEN + 0x100];
        buf[60..64].copy_from_slice(&(PE as u32).to_le_bytes());
        buf[PE..PE + 4].copy_from_slice(b"PE\0\0");
        buf[PE + 6..PE + 8].copy_from_slice(&1u16.to_le_bytes());
        buf[PE + 20..PE + 22].copy_from_slice(&(OPTIONAL as u16).to_le_bytes());
        buf[PE + 48..PE + 56].copy_from_slice(&BASE.to_le_bytes());
        let sh = PE + 24 + OPTIONAL;
        buf[sh + 12..sh + 16].copy_from_slice(&rva.to_le_bytes());
        buf[sh + 16..sh + 20].copy_from_slice(&(GATE_REGION_LEN as u32).to_le_bytes());
        buf[sh + 20..sh + 24].copy_from_slice(&(RAW_PTR as u32).to_le_bytes());
        for (i, slot) in buf[RAW_PTR..RAW_PTR + GATE_REGION_LEN]
            .iter_mut()
            .enumerate()
        {
            *slot = (i % 251) as u8;
        }
        buf
    }

    #[test]
    fn gate_key_is_the_region_digest_and_not_the_whole_file_hash() {
        let buf = region_pe();
        let region_digest: [u8; 32] = Sha256::digest(&buf[0x400..0x400 + GATE_REGION_LEN]).into();
        let whole_file: [u8; 32] = Sha256::digest(&buf).into();
        assert_eq!(
            gate_key(&buf),
            region_digest,
            "the key is the cipher region's digest"
        );
        assert_ne!(
            gate_key(&buf),
            whole_file,
            "the key is not the public whole-file hash"
        );
    }

    #[test]
    fn gate_key_tracks_the_region_and_not_bytes_outside_it() {
        let mut buf = region_pe();
        let before = gate_key(&buf);
        buf[0] ^= 0xFF; // in the DOS stub, outside the cipher region
        assert_eq!(
            gate_key(&buf),
            before,
            "a byte outside the region leaves the key alone"
        );
        buf[0x400 + 0x100] ^= 0xFF; // inside the mapped region
        assert_ne!(
            gate_key(&buf),
            before,
            "a byte inside the region changes the key"
        );
    }

    /// A PS5 SELF whose one segment maps [`SELF_GATE_REGION_VA`] to a few KiB of file bytes, the
    /// SELF counterpart of [`region_pe`].
    fn region_self() -> Vec<u8> {
        const OFFSET: usize = 0x400;
        let mut out = vec![0u8; OFFSET + GATE_REGION_LEN];
        out[..4].copy_from_slice(&super::super::gta6_image::SELF_MAGIC);
        out[0x18..0x1A].copy_from_slice(&1u16.to_le_bytes());
        out[0x20..0x28].copy_from_slice(&0x2804u64.to_le_bytes()); // phdr 0, payload
        out[0x28..0x30].copy_from_slice(&(OFFSET as u64).to_le_bytes());
        out[0x30..0x38].copy_from_slice(&(GATE_REGION_LEN as u64).to_le_bytes());
        out[0x38..0x40].copy_from_slice(&(GATE_REGION_LEN as u64).to_le_bytes());
        let elf = 0x40;
        out[elf..elf + 4].copy_from_slice(b"\x7fELF");
        out[elf + 4] = 2;
        out[elf + 5] = 1;
        out[elf + 32..elf + 40].copy_from_slice(&0x40u64.to_le_bytes()); // e_phoff
        out[elf + 54..elf + 56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
        out[elf + 56..elf + 58].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
        let ph = elf + 0x40;
        out[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        out[ph + 16..ph + 24].copy_from_slice(&SELF_GATE_REGION_VA.to_le_bytes());
        out[ph + 32..ph + 40].copy_from_slice(&(GATE_REGION_LEN as u64).to_le_bytes());
        for (i, slot) in out[OFFSET..OFFSET + GATE_REGION_LEN].iter_mut().enumerate() {
            *slot = (i % 251) as u8;
        }
        out
    }

    #[test]
    fn gate_key_of_a_self_keys_on_its_own_region() {
        let buf = region_self();
        let region_digest: [u8; 32] = Sha256::digest(&buf[0x400..0x400 + GATE_REGION_LEN]).into();
        assert_eq!(
            gate_key(&buf),
            region_digest,
            "a SELF keys on the PlayStation 5 region"
        );
        let mut other = buf.clone();
        other[0x400 + 0x100] ^= 0xFF; // inside the mapped region
        assert_ne!(gate_key(&other), gate_key(&buf));
    }

    #[test]
    fn pe_read_maps_a_virtual_address_to_its_file_bytes() {
        let (buf, va) = minimal_pe();
        let want: Vec<u8> = (0..0x20u8).map(|i| 0x40 + i).collect();
        assert_eq!(pe_read(&buf, va, 0x20), Some(&want[..]));
    }

    #[test]
    fn pe_read_refuses_what_no_section_covers() {
        let (buf, va) = minimal_pe();
        // An address below the image base, one no section maps, and a window past the section end.
        assert_eq!(pe_read(&buf, 0, 0x20), None);
        assert_eq!(pe_read(&buf, 0x1_5000_0000, 0x20), None);
        assert_eq!(pe_read(&buf, va, 0x4000), None);
        // A truncated and a non-PE input are refused, not panicked.
        assert_eq!(pe_read(&buf[..32], va, 0x20), None);
        assert_eq!(pe_read(&[0u8; 4], va, 0x20), None);
    }

    #[test]
    fn pe_read_tracks_the_mapped_window_and_not_bytes_outside_it() {
        let (mut buf, va) = minimal_pe();
        let before = pe_read(&buf, va, 0x20).unwrap().to_vec();
        buf[0x000] ^= 0xFF; // outside the window
        assert_eq!(pe_read(&buf, va, 0x20).unwrap(), &before[..]);
        buf[0x510] ^= 0xFF; // inside the window
        assert_ne!(pe_read(&buf, va, 0x20).unwrap(), &before[..]);
    }

    #[test]
    fn encode_then_decode_round_trips_under_the_same_executable() {
        let exe = b"stand-in-executable-A";
        let mut plain = PLAIN_MAGIC.to_vec();
        plain.extend(0..100u8);
        let masked = encode_masked(&plain, exe);
        assert!(is_masked(&masked));
        assert_ne!(
            &masked[8..],
            &plain[..],
            "the payload must be masked, not stored"
        );
        assert_eq!(decode_masked(&masked, exe).unwrap(), plain);
    }

    #[test]
    fn a_wrong_executable_fails_rather_than_returning_garbage() {
        let plain = {
            let mut p = PLAIN_MAGIC.to_vec();
            p.extend(0..64u8);
            p
        };
        let masked = encode_masked(&plain, b"the real executable");
        assert!(matches!(
            decode_masked(&masked, b"a different executable"),
            Err(IoError::Invalid { .. })
        ));
    }

    #[test]
    fn plain_bytes_are_not_a_masked_container() {
        let plain = PLAIN_MAGIC.to_vec();
        assert!(!is_masked(&plain));
        assert!(matches!(
            decode_masked(&plain, b"anything"),
            Err(IoError::Invalid { .. })
        ));
    }

    /// Pins the fallback key, keystream and framing to a known answer, so the Rust path and the
    /// generator that writes the committed masked snapshot cannot drift apart. The vector is from
    /// public synthetic input, never the real executable.
    #[test]
    fn the_masking_matches_a_fixed_known_answer() {
        let exe = b"KAT-exe";
        // gate_key of a non-PE input is SHA-256 of the whole input.
        assert_eq!(
            gate_key(exe),
            hex32("f471e7b811f7d408c45f468b48cfc735278c0001988a25f81dad0109de6e268f"),
        );
        let mut plain = PLAIN_MAGIC.to_vec();
        plain.extend([0u8; 40]);
        let masked = encode_masked(&plain, exe);
        assert_eq!(hex(&masked), KAT_MASKED);
        assert_eq!(decode_masked(&masked, exe).unwrap(), plain);
    }

    const KAT_MASKED: &str = "4f5247544156494dd0341dd687c54023536fed1289b90bec9824996f60524035306e4e895414b20915fd33a1c02652abc3df3b4c3eeeb9d0";

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn hex32(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }
}
