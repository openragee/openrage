//! Identity of a flat drawable; its section boundary still comes from the archive table.

use super::bytes::sections;
use super::drawable::DRAWABLE_ROOT;
use super::{HeaderlessError, Result};
use crate::container::rpf::{Rpf9Role, Rpf9Signature};

/// The role and internal name hash of a flat drawable, without inferred section sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawDrawableHint {
    pub role: Rpf9Role,
    pub name64: u64,
}

/// Recognise a flat drawable at byte zero and read the hash of its stated name.
pub fn raw_drawable_hint(bytes: &[u8]) -> Result<Option<RawDrawableHint>> {
    if bytes.starts_with(b"RSC7") {
        return Ok(None);
    }
    // Classify pointer tags first, then check their actual bounds; a broken pointer must not
    // turn a recognisable GTA VI root into an attempted GTA V resource.
    let Some(signature) = Rpf9Signature::of(bytes, 0x1000_0000) else {
        return Ok(None);
    };
    if signature.role() != Some(Rpf9Role::Drawable) {
        return Ok(None);
    }
    let name64 = sections(bytes, None, |v, _| {
        v.get("drawable root", 0, DRAWABLE_ROOT)?;
        let map_at = v.to("block map", 0x08)?;
        let shader_at = v.to("shader group", 0x10)?;
        if map_at < DRAWABLE_ROOT || map_at % 16 != 0 || shader_at < DRAWABLE_ROOT {
            return Err(HeaderlessError::Layout {
                shape: "flat drawable",
                why:
                    "its block map or shader group overlaps the root, or its block map is unaligned",
            });
        }
        // A flat export has no framing to reconstruct from these counts or slots; the
        // matching table entry supplies the section sizes before the resource is opened.
        v.get("block map", map_at, 16)?;
        let name_at = v.to("name", 0xA8)?;
        if name_at < DRAWABLE_ROOT {
            return Err(HeaderlessError::Layout {
                shape: "flat drawable",
                why: "its name overlaps the root",
            });
        }
        let rest = v.get("name", name_at, (v.len() - name_at).min(4096))?;
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(HeaderlessError::Layout {
                shape: "flat drawable",
                why: "its name has no terminator within 4096 bytes",
            })?;
        let name = std::str::from_utf8(&rest[..end]).map_err(|_| HeaderlessError::Layout {
            shape: "flat drawable",
            why: "its name is not UTF-8",
        })?;
        let lower = name.to_ascii_lowercase();
        let stem = lower
            .strip_suffix(".#dr")
            .filter(|stem| !stem.is_empty())
            .ok_or(HeaderlessError::Layout {
                shape: "flat drawable",
                why: "its name has no nonempty drawable stem ending in .#dr",
            })?;
        Ok(crate::gta6::hash::name_hash64(stem.as_bytes()))
    })?;
    Ok(Some(RawDrawableHint {
        role: Rpf9Role::Drawable,
        name64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw() -> Vec<u8> {
        let mut bytes = vec![0; 0x180];
        for (at, target) in [(0x08, 0xE0), (0x10, 0x100), (0x50, 0x130), (0xA8, 0x140)] {
            bytes[at..at + 8].copy_from_slice(&(0x5000_0000u64 + target).to_le_bytes());
        }
        bytes[0xE8] = 1;
        let name = b"Preview.#DR\0";
        bytes[0x140..0x140 + name.len()].copy_from_slice(name);
        bytes
    }

    #[test]
    fn flat_drawable_identity_uses_its_internal_stem_without_a_section_guess() {
        let hint = raw_drawable_hint(&raw()).unwrap().unwrap();
        assert_eq!(hint.role, Rpf9Role::Drawable);
        assert_eq!(hint.name64, crate::gta6::hash::name_hash64(b"preview"));
        assert_ne!(hint.name64, crate::gta6::hash::name_hash64(b"preview.ydr"));
        assert_ne!(hint.name64, crate::gta6::hash::name_hash64(b"preview.#dr"));
    }

    #[test]
    fn rsc7_and_non_root_bytes_never_enter_flat_drawable_recognition() {
        let mut rsc7 = raw();
        rsc7[..4].copy_from_slice(b"RSC7");
        assert_eq!(raw_drawable_hint(&rsc7).unwrap(), None);
        let mut prefixed = vec![0xFF; 16];
        prefixed.extend(raw());
        assert_eq!(raw_drawable_hint(&prefixed).unwrap(), None);
        assert_eq!(raw_drawable_hint(&[]).unwrap(), None);
        assert_eq!(raw_drawable_hint(&[0; 0x180]).unwrap(), None);
    }

    #[test]
    fn block_counts_and_slots_do_not_claim_a_loose_files_section_sizes() {
        let original = raw();
        let expected = raw_drawable_hint(&original).unwrap();
        let mut bytes = original;
        bytes[0xE8] = 3;
        bytes[0xE9] = 4;
        bytes[0xF0..0xF8].copy_from_slice(&0x5000_0140u64.to_le_bytes());
        assert_eq!(raw_drawable_hint(&bytes).unwrap(), expected);
    }

    #[test]
    fn a_recognisable_flat_drawable_refuses_bad_pointers_and_names() {
        for (at, pointer) in [
            (0x08, 0x5000_0180u64),
            (0x10, 0x5000_0180),
            (0xA8, 0x5000_0180),
            (0x08, 0x5000_0020),
            (0x08, 0x5000_00E1),
            (0x10, 0x5000_0020),
            (0xA8, 0x5000_0020),
        ] {
            let mut bytes = raw();
            bytes[at..at + 8].copy_from_slice(&pointer.to_le_bytes());
            assert!(raw_drawable_hint(&bytes).is_err(), "pointer at {at:#x}");
        }
        assert!(raw_drawable_hint(&raw()[..0xB0]).is_err());
        let mut bytes = raw();
        bytes[0x140..].fill(b'a');
        assert!(raw_drawable_hint(&bytes).is_err());
        let mut bytes = raw();
        bytes[0x140] = 0xFF;
        assert!(raw_drawable_hint(&bytes).is_err());
        let mut bytes = raw();
        bytes[0x149] = b't';
        assert!(raw_drawable_hint(&bytes).is_err());
    }
}
