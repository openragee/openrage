//! The machine a file or a build was made for, and RAGE's letter for it.

/// RAGE's one-letter platform prefixes (`y` PC x64, `z` Xbox Series, `p` PS5, `x` Xbox 360, …).
pub const RAGE_PLATFORM_LETTERS: &[u8] = b"ywxdzcopasnvi";

/// The machine a file or a build was made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Pc,
    Ps4,
    XboxOne,
    Ps5,
    XboxSeries,
}

impl Platform {
    pub fn label(self) -> &'static str {
        match self {
            Platform::Pc => "PC",
            Platform::Ps4 => "PlayStation 4",
            Platform::XboxOne => "Xbox One",
            Platform::Ps5 => "PlayStation 5",
            Platform::XboxSeries => "Xbox Series",
        }
    }

    /// RAGE's one-letter filename prefix for this platform: a stored `#td` becomes
    /// `ytd` on PC, `ztd` on Xbox Series, `ptd` on PS5. The letters, from the
    /// rage.re public reference:
    ///
    /// | Prefix | Platform                                 |
    /// |--------|------------------------------------------|
    /// | `y`    | Windows 64 / Linux                       |
    /// | `w`    | Windows 32 / x86                         |
    /// | `x`    | Xbox 360 (Xenon)                         |
    /// | `d`    | Xbox One (Durango)                       |
    /// | `z`    | Xbox Series (Scarlett)                   |
    /// | `c`    | PlayStation 3 / PSN                      |
    /// | `o`    | PlayStation 4 (Orbis)                    |
    /// | `p`    | PlayStation 5 (Prospero)                 |
    /// | `a`    | Android                                  |
    /// | `s`    | iOS                                      |
    /// | `n`    | Switch (NX64)                            |
    /// | `v`    | Vita (PSP2)                              |
    /// | `i`    | independent (North-specific)             |
    pub fn extension_letter(self) -> char {
        match self {
            Platform::Pc => 'y',
            Platform::XboxOne => 'd',
            Platform::XboxSeries => 'z',
            Platform::Ps4 => 'o',
            Platform::Ps5 => 'p',
        }
    }

    /// The platform a RAGE letter names; `None` for a letter of a machine no build is read of.
    pub fn from_letter(letter: u8) -> Option<Self> {
        const ALL: [Platform; 5] = [
            Platform::Pc,
            Platform::Ps4,
            Platform::XboxOne,
            Platform::Ps5,
            Platform::XboxSeries,
        ];
        ALL.into_iter()
            .find(|p| p.extension_letter() as u32 == u32::from(letter))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_platform_is_found_again_by_its_letter_and_every_letter_is_a_rage_letter() {
        for platform in [
            Platform::Pc,
            Platform::Ps4,
            Platform::XboxOne,
            Platform::Ps5,
            Platform::XboxSeries,
        ] {
            let letter = platform.extension_letter() as u8;
            assert!(RAGE_PLATFORM_LETTERS.contains(&letter), "{platform:?}");
            assert_eq!(Platform::from_letter(letter), Some(platform));
        }
        // A RAGE letter of a machine no build is read of, and a byte that is no letter.
        assert_eq!(Platform::from_letter(b'x'), None);
        assert_eq!(Platform::from_letter(0xFF), None);
    }
}
