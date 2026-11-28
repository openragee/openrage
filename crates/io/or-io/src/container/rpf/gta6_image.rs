//! The PS5 module container a GTA VI `eboot.bin` is: an unencrypted SELF (magic `54 14 F5 EE`)
//! around an x86-64 ELF, whose segment table names, for each ELF program header it carries, where
//! that header's bytes lie in the file. Only the mapping a static read needs is implemented: never
//! decryption, never compression.

/// The magic a PS5 certified file (SELF) opens with.
pub const SELF_MAGIC: [u8; 4] = [0x54, 0x14, 0xF5, 0xEE];

/// The ELF program-header type a loadable segment carries.
const PT_LOAD: u32 = 1;

/// Whether `bytes` opens as a PS5 SELF container.
pub fn is_self(bytes: &[u8]) -> bool {
    bytes.get(..4) == Some(&SELF_MAGIC[..])
}

/// The `len` bytes at virtual address `va` of the ELF the SELF in `bytes` carries, mapped through
/// its segment table: each entry with no metadata bits in its flags holds the bytes of program
/// header `(flags >> 20) & 0xfff`, and a loadable header maps `[vaddr, vaddr + filesz)` to
/// `[offset, offset + size)`. `None` on any malformed header or an address no segment covers.
pub fn self_map(bytes: &[u8], va: u64, len: usize) -> Option<&[u8]> {
    if !is_self(bytes) {
        return None;
    }
    let entries = u16_at(bytes, 0x18)? as usize;
    // The embedded ELF header starts right after the segment table.
    let elf = 32 + entries.checked_mul(32)?;
    if bytes.get(elf..elf + 4)? != b"\x7fELF" {
        return None;
    }
    // A 64-bit little-endian ELF: program headers at e_phoff, of e_phentsize bytes each.
    let class = *bytes.get(elf + 4)?;
    let data = *bytes.get(elf + 5)?;
    if class != 2 || data != 1 {
        return None;
    }
    let phoff = u64_at(bytes, elf + 32)? as usize;
    let phentsize = u16_at(bytes, elf + 54)? as usize;
    let phnum = u16_at(bytes, elf + 56)? as usize;
    if phentsize != 56 {
        return None;
    }
    let end = va.checked_add(len as u64)?;
    for i in 0..entries {
        let entry = 0x20 + i.checked_mul(32)?;
        let flags = u64_at(bytes, entry)?;
        // Entries with metadata bits name a digest of another segment, not payload bytes.
        if flags & 0x30000 != 0 {
            continue;
        }
        let offset = u64_at(bytes, entry + 8)?;
        let stored = u64_at(bytes, entry + 16)?;
        let unpacked = u64_at(bytes, entry + 24)?;
        let index = ((flags >> 20) & 0xFFF) as usize;
        let Some(header) = phdr_at(bytes, elf + phoff, phnum, index) else {
            continue;
        };
        let (kind, _, _, vaddr, _, filesz, _, _) = header;
        if kind != PT_LOAD || filesz != unpacked {
            continue;
        }
        if vaddr <= va && end <= vaddr.checked_add(filesz)? {
            let begin = usize::try_from(offset.checked_add(va - vaddr)?).ok()?;
            let seg_end = usize::try_from(offset.checked_add(stored)?).ok()?;
            if seg_end > bytes.len() {
                continue;
            }
            return bytes.get(begin..begin.checked_add(len)?);
        }
    }
    None
}

/// The `index`-th 56-byte program header of an ELF64 at `phdrs`: `(type, flags, offset, vaddr,
/// paddr, filesz, memsz, align)`.
#[allow(clippy::type_complexity)]
fn phdr_at(
    bytes: &[u8],
    phdrs: usize,
    phnum: usize,
    index: usize,
) -> Option<(u32, u32, u64, u64, u64, u64, u64, u64)> {
    if index >= phnum {
        return None;
    }
    let at = phdrs.checked_add(index.checked_mul(56)?)?;
    let le = |bytes: &[u8], at: usize, n: usize| -> Option<u64> {
        let slice = bytes.get(at..at.checked_add(n)?)?;
        let mut raw = [0u8; 8];
        raw[..n].copy_from_slice(slice);
        Some(u64::from_le_bytes(raw))
    };
    let kind = u32::try_from(le(bytes, at, 4)?).ok()?;
    let flags = u32::try_from(le(bytes, at + 4, 4)?).ok()?;
    Some((
        kind,
        flags,
        le(bytes, at + 8, 8)?,
        le(bytes, at + 16, 8)?,
        le(bytes, at + 24, 8)?,
        le(bytes, at + 32, 8)?,
        le(bytes, at + 40, 8)?,
        le(bytes, at + 48, 8)?,
    ))
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    bytes
        .get(at..at.checked_add(2)?)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    bytes
        .get(at..at.checked_add(8)?)
        .map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal PS5 SELF: the 0x20-byte header, one segment entry carrying the bytes of an ELF
    /// program header that maps `[va, va + size)` to `offset`, the embedded ELF header, its one
    /// program header, and the payload at `offset`.
    fn minimal_self(va: u64, size: usize) -> Vec<u8> {
        let offset = 0x400;
        let mut out = vec![0u8; offset + size];
        out[..4].copy_from_slice(&SELF_MAGIC);
        out[0x18..0x1A].copy_from_slice(&1u16.to_le_bytes()); // one segment entry
        let entry = 0x20;
        out[entry..entry + 8].copy_from_slice(&0x2804u64.to_le_bytes()); // phdr 0, payload
        out[entry + 8..entry + 16].copy_from_slice(&(offset as u64).to_le_bytes());
        out[entry + 16..entry + 24].copy_from_slice(&(size as u64).to_le_bytes());
        out[entry + 24..entry + 32].copy_from_slice(&(size as u64).to_le_bytes());
        let elf = 0x40;
        out[elf..elf + 4].copy_from_slice(b"\x7fELF");
        out[elf + 4] = 2; // 64-bit
        out[elf + 5] = 1; // little-endian
        out[elf + 32..elf + 40].copy_from_slice(&0x40u64.to_le_bytes()); // e_phoff
        out[elf + 54..elf + 56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
        out[elf + 56..elf + 58].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
        let ph = elf + 0x40;
        out[ph..ph + 4].copy_from_slice(&PT_LOAD.to_le_bytes());
        out[ph + 16..ph + 24].copy_from_slice(&va.to_le_bytes()); // p_vaddr
        out[ph + 32..ph + 40].copy_from_slice(&(size as u64).to_le_bytes()); // p_filesz
        for (i, slot) in out[offset..offset + size].iter_mut().enumerate() {
            *slot = (0x40 + (i % 0xC0)) as u8;
        }
        out
    }

    #[test]
    fn a_virtual_address_maps_through_the_segment_table() {
        let bytes = minimal_self(0x9B33D80, 0x200);
        let want: Vec<u8> = (0..0x20u8).map(|i| 0x40 + i).collect();
        assert_eq!(self_map(&bytes, 0x9B33D80, 0x20), Some(&want[..]));
        // The last full window inside the segment, at its own payload offset.
        let want_last: Vec<u8> = (0x1E0..0x200).map(|i| (0x40 + (i % 0xC0)) as u8).collect();
        assert_eq!(self_map(&bytes, 0x9B33F60, 0x20), Some(&want_last[..]));
    }

    #[test]
    fn what_no_segment_holds_is_refused() {
        let bytes = minimal_self(0x9B33D80, 0x200);
        assert_eq!(self_map(&bytes, 0, 0x20), None, "below the segment");
        assert_eq!(self_map(&bytes, 0x9B33F90, 0x20), None, "past its end");
        assert_eq!(self_map(&bytes, 0x9B33D80, 0x300), None, "longer than it");
    }

    #[test]
    fn a_malformed_container_is_refused_not_panicked() {
        assert_eq!(self_map(&SELF_MAGIC, 0, 0x20), None, "the magic alone");
        let mut bytes = minimal_self(0x9B33D80, 0x200);
        bytes[0x40] = 0; // the ELF magic broken
        assert_eq!(self_map(&bytes, 0x9B33D80, 0x20), None);
        let mut bytes = minimal_self(0x9B33D80, 0x200);
        bytes[0x22] = 0x10; // flags name phdr 1, which does not exist
        assert_eq!(self_map(&bytes, 0x9B33D80, 0x20), None);
        let mut bytes = minimal_self(0x9B33D80, 0x200);
        // The stored size runs past the file: the segment is unusable.
        bytes[0x20 + 16..0x20 + 24].copy_from_slice(&0xFFFFu64.to_le_bytes());
        assert_eq!(self_map(&bytes, 0x9B33D80, 0x20), None);
    }

    #[test]
    fn a_metadata_entry_carries_no_payload() {
        let mut bytes = minimal_self(0x9B33D80, 0x200);
        // A second entry describing a digest of phdr 0 overlaps the payload and must be skipped; the
        // embedded ELF moves past the grown segment table, in place so the payload stays put.
        bytes.copy_within(0x40..0xB8, 0x60);
        bytes[0x18..0x1A].copy_from_slice(&2u16.to_le_bytes());
        let entry = 0x40;
        bytes[entry..entry + 8].copy_from_slice(&(0x11u64 << 20 | 0x30000).to_le_bytes());
        bytes[entry + 8..entry + 16].copy_from_slice(&0x400u64.to_le_bytes());
        bytes[entry + 16..entry + 24].copy_from_slice(&0x200u64.to_le_bytes());
        bytes[entry + 24..entry + 32].copy_from_slice(&0x200u64.to_le_bytes());
        let want: Vec<u8> = (0..0x20u8).map(|i| 0x40 + i).collect();
        assert_eq!(self_map(&bytes, 0x9B33D80, 0x20), Some(&want[..]));
    }
}
