//! The header of RPF9, GTA VI's archive generation: 64 bytes in the clear, which state the platform
//! the archive was built for, before an enciphered table of contents and names blob.

#[cfg(feature = "gta6")]
use byteorder::{ByteOrder, LE};

#[cfg(feature = "gta6")]
use crate::error::{IoError, Result};
#[cfg(feature = "gta6")]
use crate::gta6::platform::PlatformFacts;

/// The bytes `9FPR`.
pub const RPF9_MAGIC: u32 = 0x5250_4639;
#[cfg(feature = "gta6")]
pub const RPF9_HEADER_SIZE: usize = 64;

/// The plaintext head of an RPF9 archive: the fields whose role was measured, and the 64 bytes as read.
#[cfg(feature = "gta6")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rpf9Header {
    /// The table's end; the first entry starts there, or one page on when that page is zero.
    pub toc_end: u32,
    /// The offset of the table's index, its last part, which runs to `toc_end`.
    pub index_offset: u32,
    /// The size of the encrypted names blob, which is the last thing in the archive.
    pub names_size: u32,
    /// 4 to 8; 4 puts parts B and C at 0x10 and 0x150, every value but 6 0x140 apart.
    pub layout: u8,
    /// Where the table's third part starts, counted from byte 0x40 of the header.
    pub part_c_offset: u32,
    /// 1 to 8; at 1 the index holds the entry rows alone; what higher kinds add is not known.
    pub index_kind: u16,
    /// Entries in the archive; an archive nested in it is one entry.
    pub entry_count: u32,
    /// Bytes in one entry index of the table.
    pub index_width: u8,
    /// Where the table's second part starts, counted from byte 0x40 of the header.
    pub part_b_offset: u32,
    /// Eight bytes that differ in every archive measured; what they are is not known.
    pub per_archive: [u8; 8],
    /// The build-platform letter the header states at `0x19`: `z` on Xbox Series, `p` on
    /// PlayStation 5. It is the first letter the archive's `#dr`/`#td` extensions resolve to.
    pub platform: u8,
    /// The header as read, including every word whose role is not known.
    pub raw: [u8; RPF9_HEADER_SIZE],
}

#[cfg(feature = "gta6")]
impl Rpf9Header {
    /// The header at the start of `bytes`, refused when it is short, not RPF9's, or breaks a rule
    /// every header measured holds.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let header = Self::read(bytes)?;
        match header.broken_rule() {
            None => Ok(header),
            Some(why) => Err(IoError::Invalid {
                what: "an RPF9 header".into(),
                why,
            }),
        }
    }

    /// The header at the start of `bytes`, refused only when it is short or not RPF9's.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let raw: [u8; RPF9_HEADER_SIZE] = bytes
            .get(..RPF9_HEADER_SIZE)
            .and_then(|head| head.try_into().ok())
            .ok_or(IoError::DataTooSmall {
                expected: RPF9_HEADER_SIZE,
                actual: bytes.len(),
            })?;
        let magic = LE::read_u32(&raw);
        if magic != RPF9_MAGIC {
            return Err(IoError::RpfBadMagic { found: magic });
        }
        let mut per_archive = [0u8; 8];
        per_archive.copy_from_slice(&raw[0x10..0x18]);
        Ok(Self {
            toc_end: LE::read_u32(&raw[0x04..]),
            index_offset: LE::read_u32(&raw[0x08..]),
            names_size: LE::read_u32(&raw[0x0C..]),
            layout: raw[0x1A],
            part_c_offset: LE::read_u32(&raw[0x1C..]),
            index_kind: LE::read_u16(&raw[0x22..]),
            entry_count: LE::read_u32(&raw[0x24..]),
            index_width: raw[0x2C],
            part_b_offset: LE::read_u32(&raw[0x30..]),
            per_archive,
            platform: raw[0x19],
            raw,
        })
    }

    /// The facts of the platform the header's letter states, refused for one no build was measured on.
    pub fn facts(&self) -> Result<&'static PlatformFacts> {
        PlatformFacts::of_letter(self.platform)
    }

    /// The first rule this header breaks of those every RPF9 header measured holds.
    pub fn broken_rule(&self) -> Option<String> {
        let raw = &self.raw;
        for (at, want) in [(0x18, 0), (0x1B, 0), (0x2D, 0x0F), (0x2F, 0)] {
            if raw[at] != want {
                return Some(format!(
                    "byte {at:#04X} is {:#04X}, where every header measured holds {want:#04X}",
                    raw[at]
                ));
            }
        }
        // 0x19 is the build-platform letter, and 0x2E differs by platform with it.
        let facts = match self.facts() {
            Ok(facts) => facts,
            Err(e) => return Some(format!("byte 0x19: {e}")),
        };
        if raw[0x2E] != facts.header_word {
            return Some(format!(
                "byte 0x2E is {:#04X}, where a header of {}, which the letter at 0x19 states, holds {:#04X}",
                raw[0x2E],
                facts.platform.label(),
                facts.header_word
            ));
        }
        if raw[0x34..].iter().any(|b| *b != 0) {
            return Some("bytes 0x34 to 0x3F are not zero".into());
        }
        let words = [
            ("the table's end", self.toc_end),
            ("the index offset", self.index_offset),
            ("the names size", self.names_size),
            ("part B's offset", self.part_b_offset),
            ("part C's offset", self.part_c_offset),
        ];
        if let Some((name, word)) = words.iter().find(|(_, w)| w % 16 != 0) {
            return Some(format!("{name}, {word:#X}, is not a multiple of 16"));
        }
        if !(4..=8).contains(&self.layout) {
            return Some(format!(
                "the layout byte at 0x1A is {}, not 4 to 8",
                self.layout
            ));
        }
        let (b, c) = (self.part_b_offset, self.part_c_offset);
        if (self.layout == 4) != ((c, b) == (0x150, 0x10)) {
            return Some(format!(
                "layout {} with parts B and C at {b:#X} and {c:#X}: layout 4, and it alone, puts them at 0x10 and 0x150",
                self.layout
            ));
        }
        if self.layout != 6 && c.checked_sub(b) != Some(0x140) {
            return Some(format!(
                "parts B and C at {b:#X} and {c:#X} are not 0x140 apart, as under every layout but 6"
            ));
        }
        if b > c
            || !(0x40 + u64::from(c)..u64::from(self.toc_end))
                .contains(&u64::from(self.index_offset))
        {
            return Some(format!(
                "the table's parts are out of order: B at {b:#X}, C at {c:#X}, the index at {:#X}, the end at {:#X}",
                self.index_offset, self.toc_end
            ));
        }
        let word_20 = LE::read_u16(&raw[0x20..]);
        if !(3..=13).contains(&word_20) {
            return Some(format!("the u16 at 0x20 is {word_20}, outside 3 to 13"));
        }
        if !(1..=8).contains(&self.index_kind) {
            return Some(format!(
                "the index kind at 0x22 is {}, outside 1 to 8",
                self.index_kind
            ));
        }
        let width = match self.entry_count {
            0 => return Some("the header states no entry".into()),
            1..=0xFF => 1,
            0x100..=0xFFFF => 2,
            n => return Some(format!("{n} entries, more than a 2-byte index holds")),
        };
        if self.index_width != width {
            return Some(format!(
                "{} entries with {}-byte entry indices, where the count takes {width}",
                self.entry_count, self.index_width
            ));
        }
        let rows = u64::from(self.entry_count) * (8 + u64::from(width));
        let index = u64::from(self.toc_end - self.index_offset);
        if (self.index_kind == 1 && index != rows.next_multiple_of(16))
            || (self.index_kind > 1 && index <= rows)
        {
            return Some(format!(
                "an index of {index} bytes for {rows} bytes of entry rows under index kind {}: kind 1 holds the rows alone, a higher kind more",
                self.index_kind
            ));
        }
        let on_grid = b >= 0x180 && (b - 0x10) % 0x170 == 0;
        if matches!(self.layout, 5 | 6) && self.index_kind >= 4 && !on_grid {
            return Some(format!(
                "layout {} with index kind {} and part B at {b:#X}, not 0x10 plus a whole number of 0x170",
                self.layout, self.index_kind
            ));
        }
        if self.layout == 7 && (self.index_kind != 7 || on_grid) {
            return Some(format!(
                "layout 7 with index kind {} and part B at {b:#X}: layout 7 holds kind 7 and a part B off the 0x170 grid",
                self.index_kind
            ));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::rpf::RpfArchive;
    use crate::platform::Platform;

    const TOC_END: u32 = 0x1A0;
    const NAMES: u32 = 0x30;
    const ENTRIES: u32 = 1;
    const WIDTH: u8 = 1;
    const PER_ARCHIVE: [u8; 8] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];

    /// A head with every field set to hold every measured rule, filler in the word at 0x28
    /// whose role is not known, and the bytes that would follow it: a table up to `TOC_END`
    /// and a names blob at the end.
    fn archive() -> Vec<u8> {
        let mut bytes: Vec<u8> = (0..TOC_END + NAMES).map(|i| (i * 37 + 11) as u8).collect();
        bytes[0x18..0x40].fill(0);
        bytes[0x19] = 0x7A;
        LE::write_u16(&mut bytes[0x20..], 5);
        LE::write_u32(&mut bytes[0x28..], 0x1234_5678);
        bytes[0x2D] = 0x0F;
        bytes[0x2E] = 0x09;
        LE::write_u32(&mut bytes[0x00..], RPF9_MAGIC);
        LE::write_u32(&mut bytes[0x04..], TOC_END);
        LE::write_u32(&mut bytes[0x08..], INDEX);
        LE::write_u32(&mut bytes[0x0C..], NAMES);
        bytes[0x10..0x18].copy_from_slice(&PER_ARCHIVE);
        bytes[0x1A] = LAYOUT;
        LE::write_u32(&mut bytes[0x1C..], PART_C);
        LE::write_u16(&mut bytes[0x22..], INDEX_KIND);
        LE::write_u32(&mut bytes[0x24..], ENTRIES);
        bytes[0x2C] = WIDTH;
        LE::write_u32(&mut bytes[0x30..], PART_B);
        bytes
    }

    const INDEX: u32 = 0x190;
    const LAYOUT: u8 = 4;
    const PART_C: u32 = 0x150;
    const INDEX_KIND: u16 = 1;
    const PART_B: u32 = 0x10;

    #[test]
    fn the_header_reads_the_measured_fields_and_keeps_the_rest_raw() {
        let bytes = archive();
        let header = Rpf9Header::parse(&bytes).expect("an RPF9 head");
        assert_eq!(header.toc_end, TOC_END);
        assert_eq!(header.index_offset, INDEX);
        assert_eq!(header.names_size, NAMES);
        assert_eq!(header.layout, LAYOUT);
        assert_eq!(header.part_c_offset, PART_C);
        assert_eq!(header.index_kind, INDEX_KIND);
        assert_eq!(header.part_b_offset, PART_B);
        assert_eq!(header.entry_count, ENTRIES);
        assert_eq!(header.index_width, WIDTH);
        assert_eq!(header.per_archive, PER_ARCHIVE);
        assert_eq!(header.platform, 0x7A, "the Xbox Series letter `z` at 0x19");
        assert_eq!(header.raw[..], bytes[..RPF9_HEADER_SIZE]);
        assert_eq!(&header.raw[..4], b"9FPR");

        // A PlayStation 5 header states `p` at 0x19 and its own byte at 0x2E.
        let mut ps5 = archive();
        ps5[0x19] = b'p';
        ps5[0x2E] = 0x07;
        let header = Rpf9Header::parse(&ps5).expect("a PS5 RPF9 head reads");
        assert_eq!(header.platform, b'p');
        assert_eq!(header.facts().unwrap().platform, Platform::Ps5);
    }

    #[test]
    fn a_header_that_mixes_two_platforms_or_states_an_unmeasured_one_is_refused_by_name() {
        let why = |letter: u8, word: u8| {
            let mut bytes = archive();
            bytes[0x19] = letter;
            bytes[0x2E] = word;
            match Rpf9Header::parse(&bytes) {
                Err(IoError::Invalid { why, .. }) => why,
                other => panic!("letter {letter:#04X} with {word:#04X} gave {other:?}"),
            }
        };
        // Each platform's letter with the other's byte.
        let mixed = why(b'p', 0x09);
        assert!(
            mixed.contains("0x2E") && mixed.contains("PlayStation 5"),
            "{mixed}"
        );
        let mixed = why(b'z', 0x07);
        assert!(
            mixed.contains("0x2E") && mixed.contains("Xbox Series"),
            "{mixed}"
        );
        // A platform OpenRage knows and no GTA VI build was measured on.
        let pc = why(b'y', 0x09);
        assert!(pc.contains("0x19") && pc.contains("PC"), "{pc}");
        // The lenient read still gives the header, for a scan to say what it found.
        let mut bytes = archive();
        bytes[0x19] = b'y';
        assert!(Rpf9Header::read(&bytes).unwrap().facts().is_err());
    }

    #[test]
    fn a_header_that_breaks_a_measured_rule_is_refused_naming_the_rule() {
        let u16_at = |at: usize, v: u16| move |b: &mut Vec<u8>| LE::write_u16(&mut b[at..], v);
        let u32_at = |at: usize, v: u32| move |b: &mut Vec<u8>| LE::write_u32(&mut b[at..], v);
        let byte_at = |at: usize, v: u8| move |b: &mut Vec<u8>| b[at] = v;
        type Break = Box<dyn Fn(&mut Vec<u8>)>;
        let breaks: Vec<(Break, &str)> = vec![
            (Box::new(byte_at(0x18, 1)), "0x18"),
            // 0x19 is the platform letter now; a byte that is no RAGE letter is what's refused.
            (Box::new(byte_at(0x19, 0xFF)), "0x19"),
            (Box::new(byte_at(0x1B, 1)), "0x1B"),
            (Box::new(byte_at(0x2D, 0x0E)), "0x2D"),
            (Box::new(byte_at(0x2E, 0x08)), "0x2E"),
            (Box::new(byte_at(0x2F, 1)), "0x2F"),
            (Box::new(byte_at(0x3F, 1)), "0x34 to 0x3F"),
            (Box::new(u32_at(0x0C, NAMES + 8)), "names size"),
            (Box::new(byte_at(0x1A, 3)), "0x1A"),
            (Box::new(byte_at(0x1A, 9)), "0x1A"),
            (Box::new(byte_at(0x1A, 5)), "layout 4, and it alone"),
            (Box::new(u32_at(0x30, 0x20)), "layout 4, and it alone"),
            (Box::new(u32_at(0x08, 0x180)), "out of order"),
            (Box::new(u32_at(0x08, TOC_END)), "out of order"),
            (Box::new(u16_at(0x20, 2)), "0x20"),
            (Box::new(u16_at(0x20, 14)), "0x20"),
            (Box::new(u16_at(0x22, 0)), "0x22"),
            (Box::new(u16_at(0x22, 9)), "0x22"),
            (Box::new(u32_at(0x24, 0)), "no entry"),
            (Box::new(u32_at(0x24, 0x1_0000)), "2-byte index"),
            (Box::new(byte_at(0x2C, 2)), "entry indices"),
            (Box::new(u32_at(0x24, 2)), "index kind 1"),
            // Two rows of 9 bytes fill the 16-byte index past its end: no room for more.
            (
                Box::new(|b: &mut Vec<u8>| {
                    LE::write_u16(&mut b[0x22..], 2);
                    LE::write_u32(&mut b[0x24..], 2);
                }),
                "index kind 2",
            ),
        ];
        for (break_it, named) in breaks {
            let mut bytes = archive();
            break_it(&mut bytes);
            match Rpf9Header::parse(&bytes) {
                Err(IoError::Invalid { what, why }) => {
                    assert_eq!(what, "an RPF9 header");
                    assert!(why.contains(named), "`{why}` does not name `{named}`");
                }
                other => panic!("a head that breaks `{named}` gave {other:?}"),
            }
        }
    }

    #[test]
    fn the_layout_ties_hold_for_each_layout_and_index_kind_they_name() {
        let head = |layout: u8, part_b: u32, index_kind: u16| {
            let mut bytes = archive();
            bytes[0x1A] = layout;
            LE::write_u32(&mut bytes[0x30..], part_b);
            LE::write_u32(&mut bytes[0x1C..], part_b + 0x140);
            LE::write_u16(&mut bytes[0x22..], index_kind);
            // Room for the parts and an index longer than its one row.
            LE::write_u32(&mut bytes[0x04..], 0x1000);
            LE::write_u32(&mut bytes[0x08..], 0x40 + part_b + 0x140);
            match Rpf9Header::parse(&bytes) {
                Ok(_) => None,
                Err(IoError::Invalid { why, .. }) => Some(why),
                Err(other) => panic!("{other:?}"),
            }
        };
        // Layout 7 holds kind 7 and a part B off the grid; layout 8 ties no kind to the grid.
        for (layout, part_b, kind) in [
            (5, 0x20, 2),
            (5, 0x180, 4),
            (6, 0x2F0, 5),
            (7, 0x2A0, 7),
            (7, 0x450, 7),
            (8, 0x2F0, 2),
            (8, 0x20, 4),
        ] {
            assert_eq!(
                head(layout, part_b, kind),
                None,
                "{layout} {part_b:#X} {kind}"
            );
        }
        for (layout, part_b, kind, named) in [
            (5, 0x20, 4, "layout 5 with index kind 4 and part B at 0x20"),
            (
                6,
                0x190,
                8,
                "layout 6 with index kind 8 and part B at 0x190",
            ),
            (7, 0x2A0, 5, "layout 7 with index kind 5"),
            (
                7,
                0x180,
                7,
                "layout 7 with index kind 7 and part B at 0x180",
            ),
        ] {
            let why = head(layout, part_b, kind).unwrap_or_default();
            assert!(why.contains(named), "`{why}` does not name `{named}`");
        }
        // Layout 6 alone sets its parts any distance apart.
        let mut six = archive();
        six[0x1A] = 6;
        LE::write_u32(&mut six[0x30..], 0x20);
        assert!(
            Rpf9Header::parse(&six).is_ok(),
            "{:?}",
            Rpf9Header::parse(&six)
        );
    }

    #[test]
    fn a_short_head_is_refused() {
        let bytes = archive();
        let short = &bytes[..RPF9_HEADER_SIZE - 1];
        assert!(matches!(
            Rpf9Header::parse(short),
            Err(IoError::DataTooSmall { expected: RPF9_HEADER_SIZE, actual }) if actual == short.len()
        ));
    }

    #[test]
    fn another_magic_is_refused() {
        let mut bytes = archive();
        bytes[3] = b'8';
        let found = LE::read_u32(&bytes);
        assert!(matches!(
            Rpf9Header::parse(&bytes),
            Err(IoError::RpfBadMagic { found: f }) if f == found
        ));
    }

    #[test]
    fn an_archive_is_refused_as_rpf9_with_its_table_encrypted() {
        let text = IoError::Rpf9TocEncrypted.to_string();
        for word in ["RPF9", "GTA VI", "encrypted", "no key"] {
            assert!(text.contains(word), "`{text}` lacks `{word}`");
        }
        // `from_bytes` hands the open no key session, so an RPF9 table is refused as encrypted
        // before any attempt to decipher it.
        let err = RpfArchive::from_bytes(archive()).expect_err("a filler table opens as RPF9");
        assert!(matches!(err, IoError::Rpf9TocEncrypted), "{err:?}");
    }

    #[test]
    fn an_unknown_magic_names_no_generation() {
        let mut bytes = archive();
        bytes[..4].copy_from_slice(b"XFPR");
        let err = RpfArchive::from_bytes(bytes).expect_err("no archive starts with XFPR");
        assert!(matches!(err, IoError::RpfBadMagic { .. }), "{err:?}");
        assert!(!err.to_string().contains("RPF7"), "{err}");
    }
}
