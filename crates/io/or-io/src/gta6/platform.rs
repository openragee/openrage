//! What no GTA VI file states of itself, for each platform a build was measured on.
//!
//! ⚠ A fact belongs here only when the platforms were measured to differ and no file states the
//! difference: a pack's envelope is its magic, a chunk's coder is in its row, and neither is here.

use crate::error::{IoError, Result};
use crate::platform::Platform;

/// The order a texture's blocks are laid in within a pack's image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TexelOrder {
    /// The 4 KiB and 64 KiB tiles of the Xbox Series.
    XboxTiles,
    /// The PlayStation 5's order.
    Prospero,
}

/// How the bodies of an archive are found with no table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScanStrategy {
    /// Bodies are runs of zlib chunks, found by their heads.
    ZlibHeads,
    /// Bodies are stored as they are, each from a 16-byte slot, found by their own first bytes.
    StoredBodies,
}

/// The facts of one platform's builds of GTA VI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformFacts {
    pub platform: Platform,
    /// The RPF9 header byte at `0x2E`, whose role is not known.
    pub header_word: u8,
    pub texel_order: TexelOrder,
    pub scan: ScanStrategy,
}

const FACTS: [PlatformFacts; 2] = [
    PlatformFacts {
        platform: Platform::XboxSeries,
        header_word: 0x09,
        texel_order: TexelOrder::XboxTiles,
        scan: ScanStrategy::ZlibHeads,
    },
    PlatformFacts {
        platform: Platform::Ps5,
        header_word: 0x07,
        texel_order: TexelOrder::Prospero,
        scan: ScanStrategy::StoredBodies,
    },
];

impl PlatformFacts {
    /// The facts of `platform`, refused by its name where no GTA VI build of it was measured.
    pub fn of(platform: Platform) -> Result<&'static Self> {
        FACTS
            .iter()
            .find(|facts| facts.platform == platform)
            .ok_or_else(|| IoError::Invalid {
                what: "a GTA VI platform".into(),
                why: format!("no GTA VI build for {} has been measured", platform.label()),
            })
    }

    /// The facts of the platform a file states by RAGE's letter for it.
    pub fn of_letter(letter: u8) -> Result<&'static Self> {
        match Platform::from_letter(letter) {
            Some(platform) => Self::of(platform),
            None => Err(IoError::Invalid {
                what: "a GTA VI platform".into(),
                why: format!("{letter:#04X} is the letter of no platform OpenRage reads"),
            }),
        }
    }

    /// Every platform a GTA VI build was measured on.
    pub fn measured() -> &'static [Self] {
        &FACTS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_measured_platform_has_its_facts_and_any_other_is_refused_by_name() {
        for facts in PlatformFacts::measured() {
            let letter = facts.platform.extension_letter() as u8;
            assert_eq!(PlatformFacts::of_letter(letter).unwrap(), facts);
            assert_eq!(PlatformFacts::of(facts.platform).unwrap(), facts);
        }
        let xbox = PlatformFacts::of(Platform::XboxSeries).unwrap();
        let ps5 = PlatformFacts::of(Platform::Ps5).unwrap();
        assert_ne!(xbox.header_word, ps5.header_word);
        assert_ne!(xbox.texel_order, ps5.texel_order);
        assert_ne!(xbox.scan, ps5.scan);

        let pc = PlatformFacts::of(Platform::Pc).expect_err("no PC build is measured");
        assert!(pc.to_string().contains("PC"), "{pc}");
        let xenon = PlatformFacts::of_letter(b'x').expect_err("no Xbox 360 build is read");
        assert!(xenon.to_string().contains("0x78"), "{xenon}");
    }
}
