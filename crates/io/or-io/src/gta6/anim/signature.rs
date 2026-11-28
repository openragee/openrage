//! An animation's signature at 0x2C, a Fletcher checksum over its DOFs, and a clip's at 0x38, a
//! CRC-32 over its type, its animations' signatures, windows, rates and warp lists.

use std::cell::Cell;

use super::{AnimError, Animation, Dof, Section, CLIP_ONE_LEN, ENTRY_LEN, GROUP_LEN};
use crate::decl::{Aligned, Image};

/// DOFs the sums take before both fold to 16 bits.
const RUN: usize = 180;
/// A clip's signature word.
const CLIP_SIGNATURE: usize = 0x38;
const WARP_POINT: usize = 16;

/// The reflected table of the CRC-32 polynomial 0xEDB88320.
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 == 1 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

/// The standard CRC-32 of `bytes` continued from `state`, inverted on entry and exit as zlib's.
// Chaining two calls is one CRC over both buffers.
fn crc32(state: u32, bytes: &[u8]) -> u32 {
    !bytes.iter().fold(!state, |c, &b| {
        CRC_TABLE[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8)
    })
}

/// The signature an animation of `dofs`, in their stored order, states at 0x2C.
// Per DOF the words `track << 8 | type | 0x80` and `id` go into two wrapping u32 sums from 0xFFFF.
pub fn dof_signature(dofs: &[Dof]) -> u32 {
    let fold = |sum: u32| (sum & 0xFFFF) + (sum >> 16);
    let (mut s1, mut s2) = (0xFFFFu32, 0xFFFFu32);
    for run in dofs.chunks(RUN) {
        for dof in run {
            for word in [dof.track << 8 | dof.kind | 0x80, dof.id] {
                s1 = s1.wrapping_add(word);
                s2 = s2.wrapping_add(s1);
            }
        }
        s1 = fold(s1);
        s2 = fold(s2);
    }
    fold(s2) << 16 | fold(s1)
}

impl Animation {
    /// Whether the signature at 0x2C is the one its DOFs give.
    pub fn signature_holds(&self) -> bool {
        dof_signature(&self.dofs) == self.signature
    }
}

/// Each clip's signature at 0x38 as stored and as its fields give it, in the clips' map order.
// The map order is that of `AnimResource::clips`; a root that is no clip dictionary has none.
pub fn clip_signatures(section: &[u8]) -> Result<Vec<(u32, u32)>, AnimError> {
    let aligned = Aligned::from_bytes(section);
    let signed = section_over(&aligned).signed_clips()?;
    Ok((signed.into_iter())
        .map(|(_, stored, computed)| (stored, computed))
        .collect())
}

/// The signature of each clip of `out` written at 0x38 where the fields it follows from read
/// otherwise than in `was`, the section before an edit; every other clip keeps its word as read.
pub(super) fn resign_clips(was: &[u8], out: &mut [u8]) -> Result<(), AnimError> {
    let before = Aligned::from_bytes(was);
    let after = Aligned::from_bytes(out);
    let old = section_over(&before).signed_clips()?;
    let new = section_over(&after).signed_clips()?;
    if old.len() != new.len() {
        return Err(AnimError::Unwritten(
            "the edit changed the number of clips, so no clip's signature is matched to its old one"
                .into(),
        ));
    }
    for ((_, _, then), (at, stored, now)) in old.into_iter().zip(new) {
        if then != now && stored != now {
            out[at + CLIP_SIGNATURE..at + CLIP_SIGNATURE + 4].copy_from_slice(&now.to_le_bytes());
        }
    }
    Ok(())
}

fn section_over(aligned: &Aligned) -> Section<'_> {
    let len = aligned.bytes().len();
    Section {
        image: Image::new(aligned.bytes(), len),
        left: Cell::new(len.saturating_mul(2)),
    }
}

impl Section<'_> {
    /// Each clip's offset, stored signature and computed one, in the clip map's order.
    fn signed_clips(&self) -> Result<Vec<(usize, u32, u32)>, AnimError> {
        if !matches!(self.u32("root", 0x6C)?, 2 | 3) {
            return Ok(Vec::new());
        }
        let mut clips = Vec::new();
        for (_, entry) in self.map("clip", 0x48, 0x10, true)? {
            let at = self.target("clip", entry + 8)?;
            let stored = self.u32("clip", at + CLIP_SIGNATURE)?;
            clips.push((at, stored, self.clip_signature(at)?));
        }
        Ok(clips)
    }

    /// The signature the fields of the clip at `at` give.
    // A CRC-32 from the clip's type: type 1 over its animation's signature where it points at one,
    // its start, end and rate, and its warp list's CRC; type 2 over one CRC per group entry, each
    // from 0 over the same four of the entry.
    fn clip_signature(&self, at: usize) -> Result<u32, AnimError> {
        let kind = self.u32("clip", at + 0x20)?;
        let animation = |pointer: usize| -> Result<Option<u32>, AnimError> {
            (self.pointer("clip animation", pointer)?)
                .map(|a| self.u32("animation", a + 0x2C))
                .transpose()
        };
        match kind {
            1 => {
                self.bytes("clip", at, CLIP_ONE_LEN)?;
                let mut s = kind;
                if let Some(signature) = animation(at + 0x68)? {
                    s = crc32(s, &signature.to_le_bytes());
                }
                s = crc32(s, self.bytes("clip window", at + 0x90, 12)?);
                Ok(crc32(s, &self.warp(at + 0x78)?.to_le_bytes()))
            }
            2 => {
                let mut s = kind;
                for group in self.array("clip group list", at + 0x68, GROUP_LEN)? {
                    for entry in self.array("clip entry list", group, ENTRY_LEN)? {
                        let mut e = animation(entry)?.map_or(0, |a| crc32(0, &a.to_le_bytes()));
                        e = crc32(e, self.bytes("clip entry window", entry + 0x28, 12)?);
                        e = crc32(e, &self.warp(entry + 0x10)?.to_le_bytes());
                        s = crc32(s, &e.to_le_bytes());
                    }
                }
                Ok(s)
            }
            value => Err(AnimError::ClipType {
                clip: self.name("clip name", at + 0x28)?,
                value,
            }),
        }
    }

    /// The CRC-32 of the warp list `{pointer, u16 count}` at `at`, 16 bytes a point; 0 when empty.
    fn warp(&self, at: usize) -> Result<u32, AnimError> {
        let count = usize::from(self.u16("warp point list", at + 8)?);
        if count == 0 {
            return Ok(0);
        }
        let first = self.target("warp point list", at)?;
        let points = self.bytes("warp point list", first, WARP_POINT * count)?;
        Ok(crc32(0, points))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dof(id: u32, track: u32, kind: u32) -> Dof {
        Dof { id, track, kind }
    }

    /// The sums in u64, reduced to 32 bits after every addition.
    fn reference(dofs: &[Dof]) -> u32 {
        let wrap = |v: u64| v & 0xFFFF_FFFF;
        let fold = |v: u64| (v & 0xFFFF) + (v >> 16);
        let (mut s1, mut s2) = (0xFFFFu64, 0xFFFFu64);
        for (i, d) in dofs.iter().enumerate() {
            let first = wrap((u64::from(d.track) << 8) | u64::from(d.kind) | 0x80);
            s1 = wrap(s1 + first);
            s2 = wrap(s2 + s1);
            s1 = wrap(s1 + u64::from(d.id));
            s2 = wrap(s2 + s1);
            if (i + 1) % RUN == 0 || i + 1 == dofs.len() {
                (s1, s2) = (fold(s1), fold(s2));
            }
        }
        wrap(fold(s2) << 16 | fold(s1)) as u32
    }

    /// The CRC-32 bit by bit from `state`, with no table.
    fn bitwise(state: u32, bytes: &[u8]) -> u32 {
        let mut c = !state;
        for b in bytes {
            c ^= u32::from(*b);
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    (c >> 1) ^ 0xEDB8_8320
                } else {
                    c >> 1
                };
            }
        }
        !c
    }

    #[test]
    fn the_signature_sums_each_dofs_words_and_folds_every_180() {
        assert_eq!(dof_signature(&[]), 0xFFFF_FFFF);
        let dofs: Vec<Dof> = (0..400u32)
            .map(|i| dof(0x9E37_79B9u32.wrapping_mul(i + 1), 0xC2B2_AE35 ^ i, i % 5))
            .collect();
        for n in [1, 2, 179, 180, 181, 360, 400] {
            assert_eq!(dof_signature(&dofs[..n]), reference(&dofs[..n]), "{n} DOFs");
        }
    }

    #[test]
    fn the_signature_tells_a_dof_list_from_its_reordering_and_from_another_type() {
        let list = [dof(0x11, 0x2222_0001, 0), dof(0x33, 0x4444_0002, 2)];
        let swapped = [list[1], list[0]];
        let retyped = [list[0], dof(0x33, 0x4444_0002, 3)];
        let signature = dof_signature(&list);
        assert_ne!(signature, dof_signature(&swapped));
        assert_ne!(signature, dof_signature(&retyped));
        let animation = Animation {
            dofs: list.to_vec(),
            signature,
            ..Animation::default()
        };
        assert!(animation.signature_holds());
        let other = Animation {
            signature: signature ^ 1,
            ..animation
        };
        assert!(!other.signature_holds());
    }

    #[test]
    fn the_crc_is_the_standard_one_and_chains_from_any_state() {
        let text = b"123456789";
        let mut standard = flate2::Crc::new();
        standard.update(text);
        assert_eq!(crc32(0, text), standard.sum());
        for state in [0, 1, 2, 0xDEAD_BEEF] {
            assert_eq!(crc32(state, text), bitwise(state, text), "{state:#X}");
            assert_eq!(
                crc32(crc32(state, &text[..4]), &text[4..]),
                crc32(state, text)
            );
        }
        assert_eq!(crc32(7, &[]), 7);
    }
}
