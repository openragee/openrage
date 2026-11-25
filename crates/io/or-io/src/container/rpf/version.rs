//! The archive generations Rockstar shipped, told apart by the first word of the file.

use std::fmt;

use super::header::{RpfEncryption, RPF7_MAGIC};
use crate::error::{IoError, Result};

const RSC5_MAGIC: u32 = 0x0543_5352;
const RSC6_MAGIC: u32 = 0x0643_5352;
pub(super) const RSC8_MAGIC: u32 = 0x3843_5352;

// Each entry's `tag` holds the row bits its other fields do not name:
// - RPF2, RPF3 and RPF4 resources: the resource version, the low byte of the offset word.
// - RPF6 resources: the resource version, and bit 8 for the Xenon flag of the size word.
// - RPF8, binary or resource: the encryption key id, the encryption config at bit 8 and the
//   signature flag at bit 16; a key id of `0xFF` is a body in the clear.
/// A layout of the table of contents and of what it points at, read and written through the calls RPF7 has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RpfVersion {
    /// Table Tennis.
    Rpf0,
    /// GTA IV.
    Rpf2,
    /// GTA IV's audio and Midnight Club: Los Angeles; names are hashes.
    Rpf3,
    /// Max Payne 3; offsets count eight-byte units.
    Rpf4,
    /// Red Dead Redemption, big-endian.
    Rpf6,
    /// GTA V.
    Rpf7,
    /// Red Dead Redemption 2; names are hashes, and the table has no directories.
    Rpf8,
    Rpf9,
    /// GTA III and Vice City: a `.dir` of rows beside the `.img` of bodies.
    Img1,
    /// GTA San Andreas.
    Img2,
    /// GTA IV's loose image.
    Img3,
}

impl RpfVersion {
    pub const ALL: [Self; 11] = [
        Self::Rpf0,
        Self::Rpf2,
        Self::Rpf3,
        Self::Rpf4,
        Self::Rpf6,
        Self::Rpf7,
        Self::Rpf8,
        Self::Rpf9,
        Self::Img1,
        Self::Img2,
        Self::Img3,
    ];

    /// The version a file's first four bytes announce; IMG1 has no magic, so none does.
    pub fn detect(head: [u8; 4]) -> Option<Self> {
        let magic = u32::from_le_bytes(head);
        Self::ALL.into_iter().find(|v| v.magic() == Some(magic))
    }

    pub const fn magic(self) -> Option<u32> {
        Some(match self {
            Self::Rpf0 => 0x3046_5052,
            Self::Rpf2 => 0x3246_5052,
            Self::Rpf3 => 0x3346_5052,
            Self::Rpf4 => 0x3446_5052,
            Self::Rpf6 => 0x3646_5052,
            Self::Rpf7 => RPF7_MAGIC,
            Self::Rpf8 => 0x5250_4638,
            Self::Rpf9 => super::rpf9::RPF9_MAGIC,
            Self::Img2 => 0x3252_4556,
            Self::Img3 => 0xA94E_2A52,
            Self::Img1 => return None,
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Rpf0 => "RPF0",
            Self::Rpf2 => "RPF2",
            Self::Rpf3 => "RPF3",
            Self::Rpf4 => "RPF4",
            Self::Rpf6 => "RPF6",
            Self::Rpf7 => "RPF7",
            Self::Rpf8 => "RPF8",
            Self::Rpf9 => "RPF9",
            Self::Img1 => "IMG1",
            Self::Img2 => "IMG2",
            Self::Img3 => "IMG3",
        }
    }

    /// Whether the table lists files with no directory between them.
    pub const fn is_flat(self) -> bool {
        matches!(self, Self::Rpf8 | Self::Img1 | Self::Img2 | Self::Img3)
    }

    /// Whether a name is a hash on disk and spelled `hash_XXXXXXXX` unless the archive also keeps the text.
    pub const fn hashes_names(self) -> bool {
        matches!(self, Self::Rpf3 | Self::Rpf6 | Self::Rpf8)
    }

    /// What the writer aligns every body to: the coarsest unit any of the version's offsets counts.
    pub const fn alignment(self) -> u64 {
        match self {
            Self::Rpf0 => 16,
            Self::Rpf2 | Self::Rpf3 => 256,
            Self::Rpf4 | Self::Rpf6 | Self::Img1 | Self::Img2 | Self::Img3 => 2048,
            Self::Rpf7 => 512,
            Self::Rpf8 | Self::Rpf9 => 16,
        }
    }

    /// The magic a resource of this version starts with; RPF7's is `Rsc7`'s to name.
    pub(super) const fn resource_magic(self) -> Option<u32> {
        match self {
            Self::Rpf2 | Self::Rpf3 | Self::Rpf4 | Self::Img3 => Some(RSC5_MAGIC),
            Self::Rpf6 => Some(RSC6_MAGIC),
            Self::Rpf8 => Some(RSC8_MAGIC),
            Self::Rpf0 | Self::Rpf7 | Self::Rpf9 | Self::Img1 | Self::Img2 => None,
        }
    }

    /// Whether a binary body can be deflated; the image formats keep every body as it is.
    pub(super) const fn deflates(self) -> bool {
        !matches!(self, Self::Img1 | Self::Img2 | Self::Img3)
    }

    /// The mode a header tag stands for; zero is the clear for every version but RPF8, whose clear is `0xFF`.
    pub(super) fn mode_of(self, tag: u32) -> RpfEncryption {
        match (self, tag) {
            (Self::Rpf7, raw) => RpfEncryption::from_raw(raw),
            (Self::Rpf8, 0xFF) => RpfEncryption::None,
            (Self::Rpf8, raw) => RpfEncryption::Tagged(raw),
            (_, 0) => RpfEncryption::None,
            (_, raw) => RpfEncryption::Tagged(raw),
        }
    }

    /// The header tag that writes `mode`.
    pub(super) fn tag_of(self, mode: RpfEncryption) -> Result<u32> {
        match (self, mode) {
            (Self::Rpf7, mode) => Ok(mode.to_raw()),
            (Self::Rpf8, RpfEncryption::None | RpfEncryption::Open) => Ok(0xFF),
            (_, RpfEncryption::None | RpfEncryption::Open) => Ok(0),
            (_, RpfEncryption::Tagged(raw) | RpfEncryption::Other(raw)) => Ok(raw),
            (_, mode) => Err(IoError::RpfEncryptionUnsupported {
                key_id: mode.to_raw(),
            }),
        }
    }
}

impl fmt::Display for RpfVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_magic_is_detected_as_its_own_version_and_no_other() {
        for v in RpfVersion::ALL {
            match v.magic() {
                Some(magic) => assert_eq!(RpfVersion::detect(magic.to_le_bytes()), Some(v)),
                None => assert_eq!(v, RpfVersion::Img1),
            }
        }
        assert_eq!(RpfVersion::detect(*b"\0\0\0\0"), None);
    }

    #[test]
    fn the_magic_spells_the_name_in_the_file() {
        for (v, bytes) in [
            (RpfVersion::Rpf0, b"RPF0"),
            (RpfVersion::Rpf2, b"RPF2"),
            (RpfVersion::Rpf6, b"RPF6"),
            (RpfVersion::Rpf7, b"7FPR"),
            (RpfVersion::Rpf8, b"8FPR"),
            (RpfVersion::Img2, b"VER2"),
        ] {
            assert_eq!(v.magic().map(u32::to_le_bytes), Some(*bytes), "{v}");
        }
    }

    #[test]
    fn a_tag_round_trips_to_its_mode() {
        for v in RpfVersion::ALL {
            for mode in [RpfEncryption::None, RpfEncryption::Tagged(0x2A)] {
                if v == RpfVersion::Rpf7 {
                    continue;
                }
                assert_eq!(v.mode_of(v.tag_of(mode).unwrap()), mode, "{v} {mode:?}");
            }
        }
        assert_eq!(RpfVersion::Rpf8.tag_of(RpfEncryption::None).unwrap(), 0xFF);
        assert_eq!(
            RpfVersion::Rpf8.mode_of(0),
            RpfEncryption::Tagged(0),
            "RPF8 key 0 is a key, not the clear"
        );
    }
}
